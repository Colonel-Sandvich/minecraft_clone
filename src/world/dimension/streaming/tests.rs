use std::{collections::HashSet, thread};

use avian3d::prelude::Collider;
use bevy::prelude::*;

use super::*;
use crate::{
    item::Item,
    player::{Player, PlayerDimension},
    world::{
        chunk::{
            CHUNK_SIZE, Chunk, ChunkColumn, ChunkContentCounts, ChunkHeightmap, ChunkLight,
            ChunkNeedsSave, ChunkPos, LocalBlockPos,
            mesh::{PreparedChunkMeshLight, padded_chunk_index},
        },
        definition::{
            ChunkAddress, DimensionCatalog, DimensionDefinition, DimensionId, GeneratorProfile,
        },
        dimension::{
            Active, ChunkSaveTasks, ChunkTaskPool, ColumnLightBudget, DesiredColumnView, Dimension,
            ViewDistance,
            light::{cancel_inactive_dimension_light_tasks, rebuild_chunk_light},
        },
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

fn spawn_defined_dimension(
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

fn streaming_app(height_chunks: usize) -> (App, Entity, Entity) {
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

fn staged_lighting_app(height_chunks: usize) -> (App, Entity, Entity) {
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
fn activation_stages_a_complete_pending_column_in_one_update() {
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
    assert_eq!(dimension_ref.published_chunk_count(), 0);
    let state = dimension_ref.resident_column_state(center).unwrap();
    assert!(state.is_staged());
    assert!(state.is_light_pending());
    assert_eq!(
        world.get::<Visibility>(dimension_ref.column_incarnation(center).unwrap()),
        Some(&Visibility::Hidden)
    );
    for (position, _) in dimension_ref.complete_loaded_column(center).unwrap() {
        assert!(!dimension_ref.contains_published_chunk(position));
    }
}

#[test]
fn streaming_generation_uses_the_root_dimension_definition() {
    let metadata = metadata(2);
    let definition = *DimensionCatalog::for_world(&metadata)
        .get(DimensionId::GRASS_FLOOR)
        .unwrap();
    let repository = ChunkRepository::new(NoopChunkStore::new(metadata.clone()));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(repository)
        .insert_resource(ChunkTaskPool::new_for_test())
        .init_resource::<ChunkSaveTasks>()
        .insert_resource(ColumnLoadBudget(9))
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
    let dimension = spawn_defined_dimension(&mut app, definition, true);
    app.world_mut().spawn((
        Player::default(),
        PlayerDimension::new(DimensionId::GRASS_FLOOR),
        Transform::default(),
    ));

    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .has_complete_loaded_column(ChunkColumn::new(0, 0))
    });

    let dimension = app.world().get::<Dimension>(dimension).unwrap();
    assert_eq!(dimension.id(), DimensionId::GRASS_FLOOR);
    let bottom = dimension
        .loaded_chunk_entity(ChunkPos::new(0, 0, 0))
        .and_then(|entity| app.world().get::<Chunk>(entity))
        .unwrap();
    let upper = dimension
        .loaded_chunk_entity(ChunkPos::new(0, 1, 0))
        .and_then(|entity| app.world().get::<Chunk>(entity))
        .unwrap();
    assert_eq!(bottom.cell_xyz(0, 0, 0).as_block(), Some(Item::Grass));
    assert!(bottom.cell_xyz(0, 1, 0).is_empty());
    assert!(upper.cell_xyz(0, 0, 0).is_empty());
}

#[test]
#[should_panic(expected = "dimension root definition must match the repository catalog")]
fn streaming_rejects_a_same_id_definition_that_is_not_from_the_catalog() {
    let metadata = metadata(2);
    let repository = ChunkRepository::new(NoopChunkStore::new(metadata.clone()));
    let catalog_definition = *repository.catalog().get(DimensionId::GRASS_FLOOR).unwrap();
    let mismatched = DimensionDefinition::new(
        DimensionId::GRASS_FLOOR,
        catalog_definition.height(),
        GeneratorProfile::CenterGlassPlatformV1,
        catalog_definition.arrival(),
    );
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(repository)
        .insert_resource(ChunkTaskPool::new_for_test())
        .init_resource::<ChunkSaveTasks>()
        .insert_resource(ColumnLoadBudget(1))
        .add_systems(Update, start_column_loads);
    spawn_defined_dimension(&mut app, mismatched, true);

    app.update();
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
        9
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
fn detached_saves_block_reloading_their_dimension() {
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, _) = streaming_app(2);
    app.world_mut()
        .resource_mut::<ChunkSaveTasks>()
        .retain_detached_snapshot_for_test(ChunkAddress::new(
            DimensionId::OVERWORLD,
            center.chunk(0),
        ));

    app.update();
    {
        let dimension = app.world().get::<Dimension>(dimension).unwrap();
        assert_eq!(dimension.stream().loading_count(), 0);
        assert!(dimension.stream().is_empty());
    }

    app.world_mut().insert_resource(ChunkSaveTasks::default());
    app.update();
    assert_eq!(
        app.world()
            .get::<Dimension>(dimension)
            .unwrap()
            .stream()
            .loading_count(),
        9
    );
}

#[test]
fn draining_load_attempts_is_idempotent_and_preserves_ticket_monotonicity() {
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, _) = streaming_app(2);

    app.update();
    let first = {
        let dimension = app.world().get::<Dimension>(dimension).unwrap();
        let Some(ColumnResidency::Loading { ticket, .. }) = dimension.stream().state(center) else {
            panic!("center must have an active bootstrap load");
        };
        *ticket
    };
    {
        let mut dimension = app.world_mut().get_mut::<Dimension>(dimension).unwrap();
        assert!(dimension.drain_streamed_columns().is_empty());
        assert!(dimension.stream().is_empty());
        assert!(dimension.drain_streamed_columns().is_empty());
    }

    app.update();
    let dimension = app.world().get::<Dimension>(dimension).unwrap();
    let Some(ColumnResidency::Loading { ticket: fresh, .. }) = dimension.stream().state(center)
    else {
        panic!("center must restart after stream drain");
    };
    assert!(fresh.version() > first.version());
}

#[test]
fn drain_cancels_an_active_staged_light_patch_and_returns_every_incarnation() {
    let (mut app, dimension, _) = staged_lighting_app(2);
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;
    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .loaded_chunk_count()
            == 18
    });

    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    app.update();
    let (ticket, incarnations) = {
        let dimension = app.world().get::<Dimension>(dimension).unwrap();
        let ticket = dimension
            .light_tasks()
            .active_ticket()
            .expect("staged dependency closure must start an async light patch");
        let incarnations = dimension
            .stream()
            .columns()
            .map(|column| dimension.column_incarnation(column).unwrap())
            .collect::<HashSet<_>>();
        assert_eq!(incarnations.len(), 9);
        assert_eq!(dimension.published_chunk_count(), 0);
        (ticket, incarnations)
    };

    let drained = {
        let mut dimension = app.world_mut().get_mut::<Dimension>(dimension).unwrap();
        let drained = dimension.drain_streamed_columns();
        assert_eq!(dimension.light_tasks().active_ticket(), Some(ticket));
        assert_eq!(dimension.light_tasks_mut().take_cancelled_count(), 1);
        assert!(dimension.stream().light_patch_columns(ticket).is_none());
        assert!(dimension.stream().is_empty());
        assert!(dimension.derived_work.is_empty());
        drained
    };
    assert_eq!(
        drained
            .iter()
            .map(|column| column.incarnation)
            .collect::<HashSet<_>>(),
        incarnations
    );
    assert!(
        drained
            .iter()
            .all(|column| app.world().get_entity(column.incarnation).is_ok())
    );
}

#[test]
fn drain_unpublishes_and_clears_all_disposable_work() {
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, _) = staged_lighting_app(2);
    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .contains_published_chunk(center.chunk(0))
    });

    let incarnations = {
        let dimension = app.world().get::<Dimension>(dimension).unwrap();
        assert_eq!(dimension.published_chunk_count(), 2);
        assert!(!dimension.derived_work.is_empty());
        dimension
            .stream()
            .columns()
            .map(|column| dimension.column_incarnation(column).unwrap())
            .collect::<HashSet<_>>()
    };
    let drained = app
        .world_mut()
        .get_mut::<Dimension>(dimension)
        .unwrap()
        .drain_streamed_columns();
    let dimension = app.world().get::<Dimension>(dimension).unwrap();
    assert_eq!(dimension.loaded_chunk_count(), 0);
    assert_eq!(dimension.published_chunk_count(), 0);
    assert!(dimension.stream().is_empty());
    assert!(dimension.derived_work.is_empty());
    assert_eq!(
        drained
            .iter()
            .map(|column| column.incarnation)
            .collect::<HashSet<_>>(),
        incarnations
    );
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
    let root = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .column_incarnation(center)
        .unwrap();
    let chunk_entities = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .complete_loaded_column(center)
        .unwrap()
        .into_iter()
        .map(|(_, entity)| entity)
        .collect::<Vec<_>>();
    let grandchild = app
        .world_mut()
        .spawn((ChildOf(dirty), Name::new("test descendant")))
        .id();
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
    assert!(app.world().get_entity(root).is_err());
    assert!(app.world().get_entity(grandchild).is_err());
    for entity in chunk_entities {
        assert!(app.world().get_entity(entity).is_err());
    }
}

#[test]
fn initial_center_dependencies_are_admitted_as_one_column_group() {
    let height_chunks = 4;
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, _) = streaming_app(height_chunks);

    app.update();

    let dimension = app.world().get::<Dimension>(dimension).unwrap();
    assert_eq!(dimension.stream().loading_count(), 9);
    assert_eq!(dimension.loaded_chunk_count(), 0);
    for dependency in center.chebyshev_neighborhood(1) {
        assert!(matches!(
            dimension.stream().state(dependency),
            Some(ColumnResidency::Loading { .. })
        ));
    }
}

#[test]
fn complete_dependency_halo_lights_and_publishes_center_exactly_once() {
    let height_chunks = 2;
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, _) = staged_lighting_app(height_chunks);

    // Issue exactly the center's 3x3 dependency closure, then prevent the
    // scheduler from filling later support positions while those tasks finish.
    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    for expected_columns in 1..=8 {
        update_until(&mut app, |world| {
            world
                .get::<Dimension>(dimension)
                .unwrap()
                .loaded_chunk_count()
                == expected_columns * height_chunks
        });
        let world = app.world();
        let dimension_ref = world.get::<Dimension>(dimension).unwrap();
        assert_eq!(dimension_ref.published_chunk_count(), 0);
        if let Some(state) = dimension_ref.resident_column_state(center) {
            assert!(state.is_staged());
            assert!(state.is_light_pending());
        }
        let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
        assert_eq!(perf.light_patch_runs, 0);
        assert_eq!(perf.light_patch_calculation_chunks, 0);
        assert_eq!(perf.light_patch_max_calculation_chunks, 0);
        assert_eq!(perf.light_patch_scratch_chunks, 0);
        assert_eq!(perf.light_patch_committed_columns, 0);
        assert_eq!(perf.light_rebuild_targets, 0);
    }
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .loaded_chunk_count()
            == 9 * height_chunks
    });
    {
        let world = app.world();
        let dimension_ref = world.get::<Dimension>(dimension).unwrap();
        let center_state = dimension_ref.resident_column_state(center).unwrap();
        assert!(matches!(
            center_state.lighting(),
            ColumnLighting::Calculating(_)
        ));
        assert!(center_state.is_staged());
        assert_eq!(dimension_ref.published_chunk_count(), 0);
        let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
        assert_eq!(perf.light_patch_runs, 1);
        assert_eq!(perf.light_patch_committed_columns, 0);
    }
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .contains_published_chunk(center.chunk(0))
    });

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    assert_eq!(dimension_ref.loaded_chunk_count(), 9 * height_chunks);
    assert_eq!(dimension_ref.published_chunk_count(), height_chunks);
    let center_state = dimension_ref.resident_column_state(center).unwrap();
    assert!(center_state.is_lit());
    assert!(center_state.is_published());
    for dependency in center.chebyshev_neighborhood(1) {
        assert!(dimension_ref.has_complete_loaded_column(dependency));
        if dependency != center {
            let state = dimension_ref.resident_column_state(dependency).unwrap();
            assert!(state.is_staged());
            assert!(state.is_light_pending());
        }
    }
    for (position, entity) in dimension_ref.complete_loaded_column(center).unwrap() {
        let contents = world.get::<ChunkContentCounts>(entity).unwrap();
        assert_eq!(
            dimension_ref.has_pending_collider_rebuild(position),
            contents.solid > 0,
            "publication must enqueue collider work only for solid chunks"
        );
    }

    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 1);
    assert_eq!(perf.light_patch_calculation_chunks, 9 * height_chunks);
    assert_eq!(perf.light_patch_max_calculation_chunks, 9 * height_chunks);
    assert_eq!(perf.light_patch_scratch_chunks, 8 * height_chunks);
    assert_eq!(perf.light_patch_committed_columns, 1);
    assert_eq!(perf.light_rebuild_targets, 0);
    let center_revision = center_state.light_revision();

    // The next nearest resident position is outside the finalized center's
    // dependency closure. Its arrival must not reopen or relight the center.
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 1;
    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .loaded_chunk_count()
            == 10 * height_chunks
    });
    app.update();

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    assert_eq!(
        dimension_ref
            .resident_column_state(center)
            .unwrap()
            .light_revision(),
        center_revision
    );
    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 1);
    assert_eq!(perf.light_patch_calculation_chunks, 9 * height_chunks);
    assert_eq!(perf.light_patch_max_calculation_chunks, 9 * height_chunks);
    assert_eq!(perf.light_patch_scratch_chunks, 8 * height_chunks);
    assert_eq!(perf.light_patch_committed_columns, 1);
    assert_eq!(perf.light_rebuild_targets, 0);
}

#[test]
fn stale_scratch_content_rejects_the_whole_async_patch_before_retry() {
    let height_chunks = 2;
    let center = ChunkColumn::new(0, 0);
    let scratch = ChunkColumn::new(1, 0).chunk(1);
    let (mut app, dimension, _) = staged_lighting_app(height_chunks);
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;

    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .loaded_chunk_count()
            == 9 * height_chunks
    });

    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    app.update();
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;
    {
        let dimension_ref = app.world().get::<Dimension>(dimension).unwrap();
        assert!(matches!(
            dimension_ref
                .resident_column_state(center)
                .unwrap()
                .lighting(),
            ColumnLighting::Calculating(_)
        ));
    }

    let scratch_entity = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .loaded_chunk_entity(scratch)
        .unwrap();
    let old = app
        .world()
        .get::<Chunk>(scratch_entity)
        .unwrap()
        .cell_xyz(0, 15, 0);
    let replacement = if old == crate::world::chunk::ChunkCell::EMPTY {
        Item::Glowstone.into()
    } else {
        crate::world::chunk::ChunkCell::EMPTY
    };
    app.world_mut()
        .get_mut::<Chunk>(scratch_entity)
        .unwrap()
        .set_cell_xyz(0, 15, 0, replacement);

    update_until(&mut app, |world| {
        world
            .resource::<crate::world::chunk::ChunkPerfCounters>()
            .light_patch_stale_results
            == 1
    });
    {
        let world = app.world();
        let dimension_ref = world.get::<Dimension>(dimension).unwrap();
        let center_state = dimension_ref.resident_column_state(center).unwrap();
        assert!(center_state.is_light_pending());
        assert!(center_state.is_staged());
        assert_eq!(center_state.light_revision(), ColumnLightRevision::INITIAL);
        assert_eq!(dimension_ref.published_chunk_count(), 0);
        let center_entity = dimension_ref.loaded_chunk_entity(center.chunk(0)).unwrap();
        assert!(world.get::<PreparedChunkMeshLight>(center_entity).is_none());
        let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
        assert_eq!(perf.light_patch_runs, 1);
        assert_eq!(perf.light_patch_calculation_chunks, 9 * height_chunks);
        assert_eq!(perf.light_patch_scratch_chunks, 8 * height_chunks);
        assert_eq!(perf.light_patch_committed_columns, 0);
        assert_eq!(perf.light_patch_cancelled, 0);
    }

    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .contains_published_chunk(center.chunk(0))
    });
    let world = app.world();
    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 2);
    assert_eq!(perf.light_patch_calculation_chunks, 18 * height_chunks);
    assert_eq!(perf.light_patch_scratch_chunks, 16 * height_chunks);
    assert_eq!(perf.light_patch_committed_columns, 1);
    assert_eq!(perf.light_patch_stale_results, 1);
}

#[test]
fn stale_runtime_relight_preserves_published_authority_until_retry_commits() {
    let center = ChunkColumn::new(0, 0);
    let center_position = center.chunk(0);
    let scratch_position = ChunkColumn::new(1, 0).chunk(0);
    let (mut app, dimension, _) = staged_lighting_app(1);

    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .contains_published_chunk(center_position)
    });
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;

    let (center_entity, baseline_revision, baseline_light, baseline_heightmap, baseline_prepared) = {
        let world = app.world();
        let dimension_ref = world.get::<Dimension>(dimension).unwrap();
        let center_entity = dimension_ref.loaded_chunk_entity(center_position).unwrap();
        let state = dimension_ref.resident_column_state(center).unwrap();
        assert!(state.is_lit());
        assert!(state.is_published());
        (
            center_entity,
            state.light_revision(),
            world.get::<ChunkLight>(center_entity).unwrap().clone(),
            *world.get::<ChunkHeightmap>(center_entity).unwrap(),
            world
                .get::<PreparedChunkMeshLight>(center_entity)
                .unwrap()
                .data()
                .to_vec(),
        )
    };

    assert!(
        app.world_mut()
            .get_mut::<Dimension>(dimension)
            .unwrap()
            .mark_column_light_pending(center)
    );
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    app.update();
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;
    {
        let dimension_ref = app.world().get::<Dimension>(dimension).unwrap();
        let state = dimension_ref.resident_column_state(center).unwrap();
        assert!(matches!(state.lighting(), ColumnLighting::Calculating(_)));
        assert!(state.is_published());
        assert_eq!(state.light_revision(), baseline_revision);
    }

    let scratch_entity = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .loaded_chunk_entity(scratch_position)
        .unwrap();
    let old = app
        .world()
        .get::<Chunk>(scratch_entity)
        .unwrap()
        .cell_xyz(0, 8, 8);
    let replacement = if old == crate::world::chunk::ChunkCell::EMPTY {
        Item::Glowstone.into()
    } else {
        crate::world::chunk::ChunkCell::EMPTY
    };
    app.world_mut()
        .get_mut::<Chunk>(scratch_entity)
        .unwrap()
        .set_cell_xyz(0, 8, 8, replacement);

    update_until(&mut app, |world| {
        world
            .resource::<crate::world::chunk::ChunkPerfCounters>()
            .light_patch_stale_results
            == 1
    });
    {
        let world = app.world();
        let dimension_ref = world.get::<Dimension>(dimension).unwrap();
        let state = dimension_ref.resident_column_state(center).unwrap();
        assert!(state.is_light_pending());
        assert!(state.is_published());
        assert_eq!(state.light_revision(), baseline_revision);
        assert_eq!(
            dimension_ref.published_chunk_entity(center_position),
            Some(center_entity)
        );
        assert_eq!(
            world.get::<ChunkLight>(center_entity).unwrap(),
            &baseline_light
        );
        assert_eq!(
            world.get::<ChunkHeightmap>(center_entity).unwrap(),
            &baseline_heightmap
        );
        assert_eq!(
            world
                .get::<PreparedChunkMeshLight>(center_entity)
                .unwrap()
                .data(),
            baseline_prepared
        );

        let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
        assert_eq!(perf.light_patch_runs, 2);
        assert_eq!(perf.light_patch_committed_columns, 1);
        assert_eq!(perf.light_patch_stale_results, 1);
        assert_eq!(perf.light_patch_cancelled, 0);
    }

    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .resident_column_state(center)
            .is_some_and(|state| state.is_lit() && state.light_revision() != baseline_revision)
    });

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    let state = dimension_ref.resident_column_state(center).unwrap();
    assert!(state.is_lit());
    assert!(state.is_published());
    assert_ne!(state.light_revision(), baseline_revision);
    assert_eq!(
        dimension_ref.published_chunk_entity(center_position),
        Some(center_entity)
    );
    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 3);
    assert_eq!(perf.light_patch_committed_columns, 2);
    assert_eq!(perf.light_patch_stale_results, 1);
    assert_eq!(perf.light_patch_cancelled, 0);
}

#[test]
fn pure_invalidation_cancels_an_async_claim_and_retry_uses_a_new_ticket() {
    let height_chunks = 2;
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, _) = staged_lighting_app(height_chunks);
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;

    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .loaded_chunk_count()
            == 9 * height_chunks
    });

    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    app.update();
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;
    let first_ticket = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .resident_column_state(center)
        .unwrap()
        .light_patch_ticket()
        .unwrap();
    assert!(
        app.world_mut()
            .get_mut::<Dimension>(dimension)
            .unwrap()
            .mark_column_light_pending(center)
    );
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .light_tasks()
            .is_idle()
    });

    {
        let world = app.world();
        let dimension_ref = world.get::<Dimension>(dimension).unwrap();
        assert!(dimension_ref.light_tasks().is_idle());
        assert!(
            dimension_ref
                .resident_column_state(center)
                .unwrap()
                .is_light_pending()
        );
        let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
        assert_eq!(perf.light_patch_runs, 1);
        assert_eq!(perf.light_patch_cancelled, 1);
        assert_eq!(perf.light_patch_stale_results, 0);
        assert_eq!(perf.light_patch_committed_columns, 0);
    }

    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    app.update();
    let second_ticket = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .resident_column_state(center)
        .unwrap()
        .light_patch_ticket()
        .unwrap();
    assert_ne!(second_ticket, first_ticket);
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .contains_published_chunk(center.chunk(0))
    });
    let perf = app
        .world()
        .resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 2);
    assert_eq!(perf.light_patch_committed_columns, 1);
    assert_eq!(perf.light_patch_cancelled, 1);
    assert_eq!(perf.light_patch_stale_results, 0);
}

#[test]
fn deactivating_a_dimension_cancels_its_async_light_claim() {
    let height_chunks = 2;
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, _) = staged_lighting_app(height_chunks);
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;

    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .loaded_chunk_count()
            == 9 * height_chunks
    });

    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    app.update();
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;
    let ticket = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .resident_column_state(center)
        .unwrap()
        .light_patch_ticket()
        .expect("initial lighting must claim the center before deactivation");

    app.world_mut().entity_mut(dimension).remove::<Active>();
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .light_tasks()
            .is_idle()
    });

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    assert!(dimension_ref.light_tasks().is_idle());
    let center_state = dimension_ref.resident_column_state(center).unwrap();
    assert!(center_state.is_light_pending());
    assert!(center_state.is_staged());
    assert_eq!(center_state.light_revision(), ColumnLightRevision::INITIAL);
    assert_eq!(center_state.light_patch_ticket(), None);
    assert_eq!(dimension_ref.stream().light_patch_columns(ticket), None);
    assert_eq!(dimension_ref.published_chunk_count(), 0);

    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 1);
    assert_eq!(perf.light_patch_cancelled, 1);
    assert_eq!(perf.light_patch_stale_results, 0);
    assert_eq!(perf.light_patch_committed_columns, 0);
}

#[test]
fn leaving_residency_cancels_a_patch_that_reads_the_outgoing_scratch_column() {
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, player) = staged_lighting_app(1);
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;

    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .loaded_chunk_count()
            == 9
    });
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 100;
    app.update();
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;
    assert!(matches!(
        app.world()
            .get::<Dimension>(dimension)
            .unwrap()
            .resident_column_state(center)
            .unwrap()
            .lighting(),
        ColumnLighting::Calculating(_)
    ));

    app.world_mut()
        .entity_mut(player)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::X * (CHUNK_SIZE as f32 * 2.0);
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .light_tasks()
            .is_idle()
    });

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    assert!(dimension_ref.light_tasks().is_idle());
    let center_state = dimension_ref.resident_column_state(center).unwrap();
    assert!(center_state.is_staged());
    assert!(center_state.is_light_pending());
    assert_eq!(center_state.light_revision(), ColumnLightRevision::INITIAL);
    assert!(dimension_ref.stream().columns().all(|column| {
        !matches!(
            dimension_ref.stream().state(column),
            Some(ColumnResidency::Evicting {
                resident,
                ..
            }) if matches!(resident.lighting(), ColumnLighting::Calculating(_))
        )
    }));
    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 1);
    assert_eq!(perf.light_patch_cancelled, 1);
    assert_eq!(perf.light_patch_committed_columns, 0);
}

#[test]
fn initial_bootstrap_commits_nearby_columns_before_runtime_relight() {
    let height_chunks = 2;
    let (mut app, dimension, _) = streaming_app(height_chunks);
    *app.world_mut().resource_mut::<ViewDistance>() = ViewDistance::new(2);
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 25;
    app.world_mut().resource_mut::<ColumnStagingBudget>().0 = 25;
    app.world_mut().resource_mut::<ColumnActivationBudget>().0 = 25;

    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .loaded_chunk_count()
            == 25 * height_chunks
    });
    assert_eq!(
        app.world()
            .get::<Dimension>(dimension)
            .unwrap()
            .published_chunk_count(),
        0
    );

    app.insert_resource(ColumnLightBudget(25 * height_chunks))
        .init_resource::<crate::world::chunk::ChunkPerfCounters>()
        .add_systems(Update, (rebuild_chunk_light, publish_lit_columns).chain());
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .published_chunk_count()
            == 9 * height_chunks
    });

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    assert_eq!(dimension_ref.loaded_chunk_count(), 25 * height_chunks);
    assert_eq!(dimension_ref.published_chunk_count(), 9 * height_chunks);
    for x in -1..=1 {
        for z in -1..=1 {
            let state = dimension_ref
                .resident_column_state(ChunkColumn::new(x, z))
                .unwrap();
            assert!(state.is_lit());
            assert!(state.is_published());
        }
    }
    for x in -2..=2 {
        for z in -2..=2 {
            if (-1..=1).contains(&x) && (-1..=1).contains(&z) {
                continue;
            }
            let state = dimension_ref
                .resident_column_state(ChunkColumn::new(x, z))
                .unwrap();
            assert!(state.is_staged());
            assert!(state.is_light_pending());
            assert_eq!(state.light_revision(), ColumnLightRevision::INITIAL);
        }
    }

    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 2);
    assert_eq!(perf.light_patch_calculation_chunks, 34 * height_chunks);
    assert_eq!(perf.light_patch_scratch_chunks, 25 * height_chunks);
    assert_eq!(perf.light_patch_committed_columns, 9);
    assert_eq!(perf.light_patch_stale_results, 0);
    assert_eq!(perf.light_patch_cancelled, 0);
    assert_eq!(perf.light_rebuild_targets, 0);

    // Split a runtime relight so the center commits while its right-hand
    // published neighbor remains pending scratch. The center's prepared
    // render payload must use the solved scratch halo without overwriting the
    // neighbor's authoritative ChunkLight.
    let center = ChunkColumn::new(0, 0);
    let right = ChunkColumn::new(1, 0);
    let right_position = right.chunk(0);
    let emitter = LocalBlockPos::new(0, 8, 8);
    let (center_revision, right_revision, center_entity, right_entity, old_right_light) = {
        let dimension_ref = app.world().get::<Dimension>(dimension).unwrap();
        let center_entity = dimension_ref.loaded_chunk_entity(center.chunk(0)).unwrap();
        let right_entity = dimension_ref.loaded_chunk_entity(right_position).unwrap();
        (
            dimension_ref
                .resident_column_state(center)
                .unwrap()
                .light_revision(),
            dimension_ref
                .resident_column_state(right)
                .unwrap()
                .light_revision(),
            center_entity,
            right_entity,
            app.world().get::<ChunkLight>(right_entity).unwrap().clone(),
        )
    };
    app.world_mut()
        .get_mut::<Chunk>(center_entity)
        .unwrap()
        .set_cell_xyz(
            15,
            emitter.y(),
            emitter.z(),
            crate::world::chunk::ChunkCell::EMPTY,
        );
    app.world_mut()
        .get_mut::<Chunk>(right_entity)
        .unwrap()
        .set_cell_xyz(
            emitter.x(),
            emitter.y(),
            emitter.z(),
            Item::Glowstone.into(),
        );
    {
        let mut dimension_ref = app.world_mut().get_mut::<Dimension>(dimension).unwrap();
        for dirty in right.chebyshev_neighborhood(1) {
            dimension_ref.mark_column_light_pending(dirty);
        }
    }
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 9 * height_chunks;
    app.update();
    app.world_mut().resource_mut::<ColumnLightBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .resident_column_state(center)
            .is_some_and(|state| state.is_lit() && state.light_revision() != center_revision)
    });

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    let center_state = dimension_ref.resident_column_state(center).unwrap();
    let right_state = dimension_ref.resident_column_state(right).unwrap();
    assert!(center_state.is_lit());
    assert_ne!(center_state.light_revision(), center_revision);
    assert!(right_state.is_light_pending());
    assert!(right_state.is_published());
    assert_eq!(right_state.light_revision(), right_revision);
    assert_eq!(
        world.get::<ChunkLight>(right_entity).unwrap(),
        &old_right_light
    );

    assert_eq!(
        world
            .get::<ChunkLight>(center_entity)
            .unwrap()
            .block_light(LocalBlockPos::new(15, 8, 8)),
        14
    );
    let prepared = world.get::<PreparedChunkMeshLight>(center_entity).unwrap();
    let padded = padded_chunk_index(17, 9, 9);
    let packed = ((prepared.data()[padded / 4] >> ((padded % 4) * 8)) & 0xFF) as u8;
    assert_eq!(packed & 0x0F, 15);

    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 3);
    assert_eq!(perf.light_patch_calculation_chunks, 43 * height_chunks);
    assert_eq!(perf.light_patch_scratch_chunks, 33 * height_chunks);
    assert_eq!(perf.light_patch_committed_columns, 10);
    assert_eq!(perf.light_patch_stale_results, 0);
    assert_eq!(perf.light_patch_cancelled, 0);
    assert_eq!(perf.light_rebuild_targets, 0);
}

#[test]
fn outgoing_published_column_becomes_hidden_lit_support_without_relighting() {
    let center = ChunkColumn::new(0, 0);
    let (mut app, dimension, player) = staged_lighting_app(1);
    app.update();
    app.world_mut().resource_mut::<ColumnLoadBudget>().0 = 0;
    update_until(&mut app, |world| {
        world
            .get::<Dimension>(dimension)
            .unwrap()
            .contains_published_chunk(center.chunk(0))
    });

    let before = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .resident_column_state(center)
        .unwrap();
    let root = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .column_incarnation(center)
        .unwrap();
    let center_entity = app
        .world()
        .get::<Dimension>(dimension)
        .unwrap()
        .loaded_chunk_entity(center.chunk(0))
        .unwrap();
    let collider = app
        .world_mut()
        .spawn((ChildOf(center_entity), Collider::cuboid(1.0, 1.0, 1.0)))
        .id();
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::X * (CHUNK_SIZE as f32 * 2.0);
    app.update();

    let world = app.world();
    let dimension_ref = world.get::<Dimension>(dimension).unwrap();
    let after = dimension_ref.resident_column_state(center).unwrap();
    assert!(after.is_staged());
    assert!(after.is_lit());
    assert_eq!(after.light_revision(), before.light_revision());
    assert!(dimension_ref.contains_loaded_chunk(center.chunk(0)));
    assert!(!dimension_ref.contains_published_chunk(center.chunk(0)));
    assert_eq!(world.get::<Visibility>(root), Some(&Visibility::Hidden));
    assert!(
        !dimension_ref.has_pending_mesh_rebuild(center.chunk(0)),
        "hidden support must not retain visual work"
    );
    assert!(
        !dimension_ref.has_pending_collider_rebuild(center.chunk(0)),
        "hidden support must not retain collider work"
    );
    assert!(
        world.get::<Collider>(collider).is_none(),
        "hidden support must not retain collider children"
    );
    let perf = world.resource::<crate::world::chunk::ChunkPerfCounters>();
    assert_eq!(perf.light_patch_runs, 1);
    assert_eq!(perf.light_patch_calculation_chunks, 9);
    assert_eq!(perf.light_patch_max_calculation_chunks, 9);
    assert_eq!(perf.light_patch_scratch_chunks, 8);
    assert_eq!(perf.light_patch_committed_columns, 1);
    assert_eq!(perf.light_rebuild_targets, 0);

    // Returning before the evicted side of the old halo reloads must not
    // expose the retained Lit column against incomplete raw topology.
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Transform>()
        .unwrap()
        .translation = Vec3::ZERO;
    app.update();
    let dimension_ref = app.world().get::<Dimension>(dimension).unwrap();
    let center_state = dimension_ref.resident_column_state(center).unwrap();
    assert!(center_state.is_staged());
    assert!(center_state.is_lit());
    assert!(!dimension_ref.contains_published_chunk(center.chunk(0)));
    assert!(!dimension_ref.has_complete_resident_light_neighborhood(center));
}
