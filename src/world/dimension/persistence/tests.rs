use std::{
    io::ErrorKind,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use bevy::prelude::*;

use super::*;
use crate::{
    block::BlockType,
    world::{
        chunk::{ChunkHeightmap, ChunkLight},
        definition::{ChunkAddress, DimensionId},
        generation::WorldMetadata,
        storage::{ChunkRepository, ChunkStore, InMemoryChunkStore},
    },
};

fn overworld(position: ChunkPos) -> ChunkAddress {
    ChunkAddress::new(DimensionId::OVERWORLD, position)
}

fn update_until(app: &mut App, mut predicate: impl FnMut(&World) -> bool) {
    for _ in 0..2_000 {
        app.update();
        if predicate(app.world()) {
            return;
        }
        std::thread::yield_now();
    }

    panic!("condition was not met after 2,000 updates");
}

fn save_app(repository: ChunkRepository, budget: usize) -> (App, Entity) {
    let metadata = repository.metadata().clone();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(metadata.clone())
        .insert_resource(repository)
        .insert_resource(ChunkSaveBudget(budget))
        .insert_resource(ChunkTaskPool::new_for_test())
        .insert_resource(ChunkSaveTasks::default())
        .init_resource::<DesiredColumnView>()
        .add_systems(
            Update,
            (finish_chunk_save_tasks, start_chunk_save_tasks).chain(),
        );
    let owner = spawn_dimension(&mut app, metadata.height(), true);
    (app, owner)
}

fn spawn_dimension(app: &mut App, height: crate::world::WorldHeight, active: bool) -> Entity {
    let owner = app.world_mut().spawn_empty().id();
    let mut entity = app.world_mut().entity_mut(owner);
    entity.insert(Dimension::new(owner, height));
    if active {
        entity.insert(Active);
    }
    owner
}

fn spawn_dirty_chunk(
    app: &mut App,
    owner: Entity,
    position: ChunkPos,
    chunk: Chunk,
    heightmap: ChunkHeightmap,
) -> Entity {
    let entity = app
        .world_mut()
        .spawn((
            ChildOf(owner),
            ChunkPosition::from(position),
            chunk,
            ChunkLight::default(),
            heightmap,
            ChunkNeedsSave,
        ))
        .id();
    assert_eq!(
        app.world_mut()
            .get_mut::<Dimension>(owner)
            .unwrap()
            .register_published_chunk(position, entity),
        None
    );
    entity
}

fn save_handle(owner: Entity, entity: Entity, position: ChunkPos) -> ChunkSaveHandle {
    ChunkSaveHandle {
        owner,
        entity,
        position,
    }
}

fn io_error(kind: ErrorKind) -> ChunkStoreError {
    ChunkStoreError::Io {
        kind,
        message: "intentional save failure".to_owned(),
    }
}

struct ScriptedSaveStore {
    inner: InMemoryChunkStore,
    calls: Arc<AtomicUsize>,
    failures_before_success: usize,
    failure_kind: ErrorKind,
}

impl ScriptedSaveStore {
    fn new(
        metadata: WorldMetadata,
        calls: Arc<AtomicUsize>,
        failures_before_success: usize,
        failure_kind: ErrorKind,
    ) -> Self {
        Self {
            inner: InMemoryChunkStore::new(metadata),
            calls,
            failures_before_success,
            failure_kind,
        }
    }
}

impl ChunkStore for ScriptedSaveStore {
    fn metadata(&self) -> &WorldMetadata {
        self.inner.metadata()
    }

    fn load_chunk(
        &self,
        address: ChunkAddress,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        self.inner.load_chunk(address)
    }

    fn save_chunk(
        &self,
        address: ChunkAddress,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < self.failures_before_success {
            return Err(io_error(self.failure_kind));
        }
        self.inner.save_chunk(address, chunk, heightmap)
    }
}

fn candidate(handle: ChunkSaveHandle, eviction_priority: bool) -> ChunkSaveCandidate {
    ChunkSaveCandidate {
        handle,
        revision: ChunkRevision::INITIAL,
        eviction_priority,
    }
}

#[test]
fn failure_records_distinguish_retryable_and_permanent_errors() {
    let handle = save_handle(Entity::PLACEHOLDER, Entity::PLACEHOLDER, ChunkPos::ZERO);
    let mut save_tasks = ChunkSaveTasks::default();

    save_tasks.record_failure(handle, io_error(ErrorKind::TimedOut));
    assert_eq!(save_tasks.failures[&handle].attempts, 1);
    assert_eq!(
        save_tasks.failures[&handle].retry_after_updates,
        Some(retry_delay_for_attempt(1))
    );

    save_tasks.record_failure(handle, io_error(ErrorKind::TimedOut));
    assert_eq!(save_tasks.failures[&handle].attempts, 2);
    assert_eq!(
        save_tasks.failures[&handle].retry_after_updates,
        Some(retry_delay_for_attempt(2))
    );

    save_tasks.record_failure(handle, io_error(ErrorKind::PermissionDenied));
    assert_eq!(save_tasks.failures[&handle].attempts, 3);
    assert_eq!(save_tasks.failures[&handle].retry_after_updates, None);
}

#[test]
fn dirty_chunks_are_persisted_and_marked_clean() {
    let metadata = WorldMetadata::with_seed(9);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata));
    let position = ChunkPos::new(2, 0, -1);
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(0, 0, 0, BlockType::OakLog.into());
    let (mut app, owner) = save_app(repository.clone(), usize::MAX);
    let entity = spawn_dirty_chunk(
        &mut app,
        owner,
        position,
        chunk.clone(),
        ChunkHeightmap::default(),
    );

    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(entity).is_none()
    });

    let (loaded, _) = repository.load_chunk(overworld(position)).unwrap().unwrap();
    assert_eq!(loaded, chunk);
}

#[test]
fn save_budget_limits_total_in_flight_chunks() {
    let metadata = WorldMetadata::with_seed(9);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata));
    let (mut app, owner) = save_app(repository, 1);
    spawn_dirty_chunk(
        &mut app,
        owner,
        ChunkPos::ZERO,
        Chunk::default(),
        ChunkHeightmap::default(),
    );
    spawn_dirty_chunk(
        &mut app,
        owner,
        ChunkPos::new(1, 0, 0),
        Chunk::default(),
        ChunkHeightmap::default(),
    );

    app.update();

    assert_eq!(app.world().resource::<ChunkSaveTasks>().tasks.len(), 1);
}

#[test]
fn saves_are_serialized_within_a_column() {
    let metadata = WorldMetadata::with_seed(9).with_height_chunks(2).unwrap();
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata));
    let (mut app, owner) = save_app(repository.clone(), usize::MAX);
    let lower = spawn_dirty_chunk(
        &mut app,
        owner,
        ChunkPos::new(0, 0, 0),
        Chunk::default(),
        ChunkHeightmap::default(),
    );
    let upper = spawn_dirty_chunk(
        &mut app,
        owner,
        ChunkPos::new(0, 1, 0),
        Chunk::default(),
        ChunkHeightmap::default(),
    );

    app.update();

    assert_eq!(app.world().resource::<ChunkSaveTasks>().tasks.len(), 1);
    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(lower).is_none() && world.get::<ChunkNeedsSave>(upper).is_none()
    });
    assert!(
        repository
            .load_chunk(overworld(ChunkPos::new(0, 0, 0)))
            .unwrap()
            .is_some()
    );
    assert!(
        repository
            .load_chunk(overworld(ChunkPos::new(0, 1, 0)))
            .unwrap()
            .is_some()
    );
}

#[test]
fn stale_revision_completion_cannot_clean_identical_live_content() {
    let metadata = WorldMetadata::with_seed(9);
    let calls = Arc::new(AtomicUsize::new(0));
    let repository = ChunkRepository::new(ScriptedSaveStore::new(
        metadata,
        calls.clone(),
        0,
        ErrorKind::Other,
    ));
    let position = ChunkPos::new(2, 0, -1);
    let mut original = Chunk::default();
    original.set_cell_xyz(0, 0, 0, BlockType::OakLog.into());
    let (mut app, owner) = save_app(repository.clone(), usize::MAX);
    let entity = spawn_dirty_chunk(
        &mut app,
        owner,
        position,
        original.clone(),
        ChunkHeightmap::default(),
    );

    app.update();
    let first_revision = app
        .world()
        .resource::<ChunkSaveTasks>()
        .tasks
        .values()
        .next()
        .unwrap()
        .ticket
        .revision;
    {
        let mut chunk = app.world_mut().get_mut::<Chunk>(entity).unwrap();
        chunk.set_cell_xyz(0, 0, 0, BlockType::Stone.into());
        chunk.set_cell_xyz(0, 0, 0, BlockType::OakLog.into());
        assert_eq!(*chunk, original);
        assert_ne!(chunk.content_revision(), first_revision);
    }
    let live_revision = app.world().get::<Chunk>(entity).unwrap().content_revision();

    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(entity).is_some()
            && world
                .resource::<ChunkSaveTasks>()
                .tasks
                .values()
                .any(|task| task.ticket.revision == live_revision)
    });
    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(entity).is_none()
    });

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let (loaded, _) = repository.load_chunk(overworld(position)).unwrap().unwrap();
    assert_eq!(loaded, original);
}

#[test]
fn stale_heightmap_completion_cannot_clean_a_chunk() {
    let metadata = WorldMetadata::with_seed(9);
    let calls = Arc::new(AtomicUsize::new(0));
    let repository = ChunkRepository::new(ScriptedSaveStore::new(
        metadata,
        calls.clone(),
        0,
        ErrorKind::Other,
    ));
    let position = ChunkPos::ZERO;
    let (mut app, owner) = save_app(repository.clone(), usize::MAX);
    let entity = spawn_dirty_chunk(
        &mut app,
        owner,
        position,
        Chunk::default(),
        ChunkHeightmap::default(),
    );

    app.update();
    let revision = app.world().get::<Chunk>(entity).unwrap().content_revision();
    let mut current_heightmap = ChunkHeightmap::default();
    current_heightmap.heights[3][7] = 11;
    app.world_mut().entity_mut(entity).insert(current_heightmap);

    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(entity).is_some()
            && world
                .resource::<ChunkSaveTasks>()
                .tasks
                .values()
                .any(|task| {
                    task.ticket.revision == revision && task.ticket.heightmap == current_heightmap
                })
    });
    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(entity).is_none()
    });

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let (_, stored_heightmap) = repository.load_chunk(overworld(position)).unwrap().unwrap();
    assert_eq!(stored_heightmap, current_heightmap);
}

#[test]
fn only_the_active_dimensions_registered_chunks_start_saves() {
    let metadata = WorldMetadata::with_seed(9);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));
    let (mut app, active_owner) = save_app(repository, usize::MAX);
    let inactive_owner = spawn_dimension(&mut app, metadata.height(), false);
    let position = ChunkPos::ZERO;
    let active = spawn_dirty_chunk(
        &mut app,
        active_owner,
        position,
        Chunk::default(),
        ChunkHeightmap::default(),
    );
    let inactive = spawn_dirty_chunk(
        &mut app,
        inactive_owner,
        position,
        Chunk::default(),
        ChunkHeightmap::default(),
    );
    app.world_mut().spawn((
        ChunkPosition::from(ChunkPos::new(5, 0, 5)),
        Chunk::default(),
        ChunkHeightmap::default(),
        ChunkNeedsSave,
    ));

    app.update();

    let save_tasks = app.world().resource::<ChunkSaveTasks>();
    assert_eq!(save_tasks.tasks.len(), 1);
    let handle = *save_tasks.tasks.keys().next().unwrap();
    assert_eq!(handle.owner, active_owner);
    assert_eq!(handle.entity, active);
    assert_ne!(handle.entity, inactive);
}

#[test]
fn completion_stays_bound_to_its_original_owner_after_an_active_switch() {
    let metadata = WorldMetadata::with_seed(9);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));
    let (mut app, first_owner) = save_app(repository, usize::MAX);
    let second_owner = spawn_dimension(&mut app, metadata.height(), false);
    let position = ChunkPos::ZERO;
    let first = spawn_dirty_chunk(
        &mut app,
        first_owner,
        position,
        Chunk::default(),
        ChunkHeightmap::default(),
    );
    let second = spawn_dirty_chunk(
        &mut app,
        second_owner,
        position,
        Chunk::default(),
        ChunkHeightmap::default(),
    );

    app.update();
    app.world_mut().entity_mut(first_owner).remove::<Active>();
    app.world_mut().entity_mut(second_owner).insert(Active);
    app.world_mut().resource_mut::<ChunkSaveBudget>().0 = 0;
    update_until(&mut app, |world| {
        world.resource::<ChunkSaveTasks>().tasks.is_empty()
    });

    assert!(app.world().get::<ChunkNeedsSave>(first).is_none());
    assert!(app.world().get::<ChunkNeedsSave>(second).is_some());
}

#[test]
fn transient_failures_back_off_then_retry() {
    let metadata = WorldMetadata::with_seed(9);
    let calls = Arc::new(AtomicUsize::new(0));
    let repository = ChunkRepository::new(ScriptedSaveStore::new(
        metadata,
        calls.clone(),
        1,
        ErrorKind::TimedOut,
    ));
    let (mut app, owner) = save_app(repository, usize::MAX);
    let entity = spawn_dirty_chunk(
        &mut app,
        owner,
        ChunkPos::ZERO,
        Chunk::default(),
        ChunkHeightmap::default(),
    );

    update_until(&mut app, |world| {
        world
            .resource::<ChunkSaveTasks>()
            .failures
            .values()
            .any(|failure| failure.retry_after_updates.is_some())
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(entity).is_none()
    });
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn permanent_failures_do_not_retry_or_allow_eviction() {
    let metadata = WorldMetadata::with_seed(9);
    let calls = Arc::new(AtomicUsize::new(0));
    let repository = ChunkRepository::new(ScriptedSaveStore::new(
        metadata,
        calls.clone(),
        usize::MAX,
        ErrorKind::PermissionDenied,
    ));
    let (mut app, owner) = save_app(repository, usize::MAX);
    let entity = spawn_dirty_chunk(
        &mut app,
        owner,
        ChunkPos::ZERO,
        Chunk::default(),
        ChunkHeightmap::default(),
    );

    update_until(&mut app, |world| {
        world
            .resource::<ChunkSaveTasks>()
            .failures
            .values()
            .any(|failure| failure.retry_after_updates.is_none())
    });
    for _ in 0..INITIAL_SAVE_RETRY_DELAY_UPDATES * 2 {
        app.update();
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(app.world().get::<ChunkNeedsSave>(entity).is_some());
    assert!(app.world().resource::<ChunkSaveTasks>().tasks.is_empty());
}

#[test]
fn selection_is_round_robin_within_each_priority() {
    let mut world = World::new();
    let owner = world.spawn_empty().id();
    let entities = [
        world.spawn_empty().id(),
        world.spawn_empty().id(),
        world.spawn_empty().id(),
    ];
    let handles = [
        save_handle(owner, entities[0], ChunkPos::new(0, 0, 0)),
        save_handle(owner, entities[1], ChunkPos::new(1, 0, 0)),
        save_handle(owner, entities[2], ChunkPos::new(2, 0, 0)),
    ];
    let candidates = vec![
        candidate(handles[2], false),
        candidate(handles[0], false),
        candidate(handles[1], false),
    ];
    let mut cursor = None;

    for expected in [handles[0], handles[1], handles[2], handles[0]] {
        let selected = ordered_candidates(candidates.clone(), cursor)[0].handle;
        assert_eq!(selected, expected);
        cursor = Some(selected);
    }
}

#[test]
fn chunks_waiting_to_evict_are_saved_before_resident_chunks() {
    let metadata = WorldMetadata::with_seed(9);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));
    let (mut app, owner) = save_app(repository, 1);
    app.world_mut().resource_mut::<DesiredColumnView>().refresh(
        ChunkColumn::new(0, 0),
        crate::world::dimension::ViewDistance::new(1),
        metadata.height(),
    );
    spawn_dirty_chunk(
        &mut app,
        owner,
        ChunkPos::ZERO,
        Chunk::default(),
        ChunkHeightmap::default(),
    );
    let evicting_position = ChunkPos::new(10, 0, 0);
    spawn_dirty_chunk(
        &mut app,
        owner,
        evicting_position,
        Chunk::default(),
        ChunkHeightmap::default(),
    );

    app.update();

    let handle = *app
        .world()
        .resource::<ChunkSaveTasks>()
        .tasks
        .keys()
        .next()
        .unwrap();
    assert_eq!(handle.position, evicting_position);
}
