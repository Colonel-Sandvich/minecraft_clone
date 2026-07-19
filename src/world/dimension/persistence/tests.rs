use std::{
    io::ErrorKind,
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use bevy::prelude::*;

use super::*;
use crate::{
    item::Item,
    world::{
        chunk::{ChunkColumn, ChunkHeightmap, ChunkLight, ChunkPos},
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
    save_app_in_dimension(repository, budget, DimensionId::OVERWORLD)
}

fn save_app_in_dimension(
    repository: ChunkRepository,
    budget: usize,
    dimension_id: DimensionId,
) -> (App, Entity) {
    let metadata = repository.metadata().clone();
    let definition = *repository.catalog().get(dimension_id).unwrap();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(metadata.clone())
        .insert_resource(repository)
        .insert_resource(ChunkSaveBudget(budget))
        .insert_resource(ChunkTaskPool::new_for_test())
        .insert_resource(ChunkSaveTasks::default())
        .add_systems(
            Update,
            (finish_chunk_save_tasks, start_chunk_save_tasks).chain(),
        );
    let owner = spawn_defined_dimension(&mut app, definition, true);
    (app, owner)
}

fn spawn_dimension(app: &mut App, height: crate::world::WorldHeight, active: bool) -> Entity {
    let owner = app.world_mut().spawn_empty().id();
    let mut entity = app.world_mut().entity_mut(owner);
    entity.insert((
        Dimension::new_for_test(owner, height),
        DesiredColumnView::default(),
    ));
    if active {
        entity.insert(Active);
    }
    owner
}

fn spawn_defined_dimension(
    app: &mut App,
    definition: crate::world::DimensionDefinition,
    active: bool,
) -> Entity {
    let owner = app.world_mut().spawn_empty().id();
    let mut entity = app.world_mut().entity_mut(owner);
    entity.insert((
        Dimension::new(owner, definition),
        DesiredColumnView::default(),
    ));
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

struct GatedFirstSaveStore {
    inner: InMemoryChunkStore,
    calls: Arc<AtomicUsize>,
    first_started: Arc<AtomicBool>,
    release_first: Arc<AtomicBool>,
    first_failure: Option<ErrorKind>,
}

impl GatedFirstSaveStore {
    fn new(metadata: WorldMetadata, first_failure: Option<ErrorKind>) -> (Self, GatedSaveControl) {
        let calls = Arc::new(AtomicUsize::new(0));
        let first_started = Arc::new(AtomicBool::new(false));
        let release_first = Arc::new(AtomicBool::new(false));
        (
            Self {
                inner: InMemoryChunkStore::new(metadata),
                calls: calls.clone(),
                first_started: first_started.clone(),
                release_first: release_first.clone(),
                first_failure,
            },
            GatedSaveControl {
                calls,
                first_started,
                release_first,
            },
        )
    }
}

impl ChunkStore for GatedFirstSaveStore {
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
        if call == 0 {
            self.first_started.store(true, Ordering::Release);
            while !self.release_first.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            if let Some(kind) = self.first_failure {
                return Err(io_error(kind));
            }
        }
        self.inner.save_chunk(address, chunk, heightmap)
    }
}

struct GatedSaveControl {
    calls: Arc<AtomicUsize>,
    first_started: Arc<AtomicBool>,
    release_first: Arc<AtomicBool>,
}

impl GatedSaveControl {
    fn wait_until_first_started(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.first_started.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "first save did not start");
            std::thread::yield_now();
        }
    }

    fn release_first(&self) {
        self.release_first.store(true, Ordering::Release);
    }
}

impl Drop for GatedSaveControl {
    fn drop(&mut self) {
        self.release_first.store(true, Ordering::Release);
    }
}

fn capture_all_dimension_save_snapshots(
    mut save_tasks: ResMut<ChunkSaveTasks>,
    dimensions: Query<(&Dimension, &DesiredColumnView, Entity)>,
    chunks: Query<(
        &ChunkPosition,
        &Chunk,
        &ChunkHeightmap,
        Option<&ChunkNeedsSave>,
    )>,
) {
    for (dimension, desired_view, owner) in &dimensions {
        capture_dimension_save_snapshots(
            &mut save_tasks,
            dimension,
            SaveSnapshotContext::ResidentView(desired_view),
            owner,
            &chunks,
        );
    }
}

fn candidate(address: ChunkAddress, eviction_priority: bool) -> ChunkSaveCandidate {
    ChunkSaveCandidate {
        address,
        eviction_priority,
    }
}

#[test]
fn detached_capture_prioritizes_columns_that_were_resident_before_teardown() {
    let center = ChunkColumn::new(0, 0);
    let mut view = DesiredColumnView::default();
    assert!(view.refresh(
        center,
        crate::world::dimension::ViewDistance::new(1),
        WorldMetadata::default().height(),
    ));

    assert!(!SaveSnapshotContext::ResidentView(&view).eviction_priority(center));
    assert!(SaveSnapshotContext::Detached.eviction_priority(center));
}

#[test]
fn failure_records_distinguish_retryable_and_permanent_errors() {
    let address = overworld(ChunkPos::ZERO);
    let mut save_tasks = ChunkSaveTasks::default();

    save_tasks.record_failure(address, io_error(ErrorKind::TimedOut));
    assert_eq!(save_tasks.failures[&address].attempts, 1);
    assert_eq!(
        save_tasks.failures[&address].retry_after_updates,
        Some(retry_delay_for_attempt(1))
    );

    save_tasks.record_failure(address, io_error(ErrorKind::TimedOut));
    assert_eq!(save_tasks.failures[&address].attempts, 2);
    assert_eq!(
        save_tasks.failures[&address].retry_after_updates,
        Some(retry_delay_for_attempt(2))
    );

    save_tasks.record_failure(address, io_error(ErrorKind::PermissionDenied));
    assert_eq!(save_tasks.failures[&address].attempts, 3);
    assert_eq!(save_tasks.failures[&address].retry_after_updates, None);
}

#[test]
fn equal_local_columns_in_different_dimensions_have_distinct_save_lanes() {
    let position = ChunkPos::new(4, 1, -7);
    let overworld = overworld(position);
    let grass_floor = ChunkAddress::new(DimensionId::GRASS_FLOOR, position);

    assert_ne!(overworld.column(), grass_floor.column());
    assert_ne!(address_order_key(overworld), address_order_key(grass_floor));
}

#[test]
fn outgoing_dimension_capture_does_not_require_active_or_an_io_budget() {
    let metadata = WorldMetadata::with_seed(9);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(ChunkSaveTasks::default())
        .add_systems(Update, capture_all_dimension_save_snapshots);
    let owner = spawn_dimension(&mut app, metadata.height(), false);
    let position = ChunkPos::new(3, 0, -2);
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(0, 0, 0, Item::OakLog.into());
    let entity = spawn_dirty_chunk(
        &mut app,
        owner,
        position,
        chunk.clone(),
        ChunkHeightmap::default(),
    );

    app.update();

    let address = overworld(position);
    let save_tasks = app.world().resource::<ChunkSaveTasks>();
    assert!(save_tasks.pending.contains_key(&address));
    assert!(save_tasks.in_flight.is_empty());
    assert!(save_tasks.has_uncommitted_dimension(DimensionId::OVERWORLD));
    assert!(
        save_tasks.stats().estimated_payload_bytes
            >= size_of::<ChunkAddress>()
                + size_of::<PendingChunkSave>()
                + size_of::<ChunkSavePayload>()
    );

    app.world_mut().despawn(entity);
    app.world_mut().despawn(owner);
    assert_eq!(
        app.world().resource::<ChunkSaveTasks>().pending[&address]
            .snapshot
            .payload
            .chunk,
        chunk
    );
}

#[test]
fn transient_failure_retries_owned_snapshot_after_root_despawn() {
    let metadata = WorldMetadata::with_seed(9);
    let (store, control) = GatedFirstSaveStore::new(metadata, Some(ErrorKind::TimedOut));
    let repository = ChunkRepository::new(store);
    let position = ChunkPos::new(2, 0, -1);
    let address = overworld(position);
    let mut expected = Chunk::default();
    expected.set_cell_xyz(0, 0, 0, Item::OakLog.into());
    let mut heightmap = ChunkHeightmap::default();
    heightmap.heights[2][5] = 9;
    let (mut app, owner) = save_app(repository.clone(), usize::MAX);
    let entity = spawn_dirty_chunk(&mut app, owner, position, expected.clone(), heightmap);

    app.update();
    control.wait_until_first_started();
    assert!(
        app.world()
            .resource::<ChunkSaveTasks>()
            .in_flight
            .contains_key(&address)
    );

    app.world_mut().despawn(entity);
    app.world_mut().despawn(owner);
    control.release_first();

    update_until(&mut app, |world| {
        !world
            .resource::<ChunkSaveTasks>()
            .has_uncommitted_dimension(DimensionId::OVERWORLD)
    });

    assert_eq!(control.calls.load(Ordering::SeqCst), 2);
    let (stored, stored_heightmap) = repository.load_chunk(address).unwrap().unwrap();
    assert_eq!(stored, expected);
    assert_eq!(stored_heightmap, heightmap);
}

#[test]
fn newer_live_revision_coalesces_behind_in_flight_snapshot() {
    let metadata = WorldMetadata::with_seed(9);
    let (store, control) = GatedFirstSaveStore::new(metadata, None);
    let repository = ChunkRepository::new(store);
    let position = ChunkPos::new(2, 0, -1);
    let address = overworld(position);
    let mut original = Chunk::default();
    original.set_cell_xyz(0, 0, 0, Item::OakLog.into());
    let (mut app, owner) = save_app(repository.clone(), usize::MAX);
    let entity = spawn_dirty_chunk(
        &mut app,
        owner,
        position,
        original,
        ChunkHeightmap::default(),
    );

    app.update();
    control.wait_until_first_started();
    let first_revision = app.world().get::<Chunk>(entity).unwrap().content_revision();
    app.world_mut()
        .get_mut::<Chunk>(entity)
        .unwrap()
        .set_cell_xyz(1, 0, 0, Item::Stone.into());
    let expected = app.world().get::<Chunk>(entity).unwrap().clone();
    let latest_revision = expected.content_revision();

    app.update();

    let save_tasks = app.world().resource::<ChunkSaveTasks>();
    assert_eq!(
        save_tasks.in_flight[&address]
            .snapshot
            .source
            .unwrap()
            .revision,
        first_revision
    );
    assert_eq!(
        save_tasks.pending[&address]
            .snapshot
            .source
            .unwrap()
            .revision,
        latest_revision
    );
    assert_eq!(save_tasks.stats().tasks, 2);

    control.release_first();
    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(entity).is_none()
    });

    assert_eq!(control.calls.load(Ordering::SeqCst), 2);
    let (stored, _) = repository.load_chunk(address).unwrap().unwrap();
    assert_eq!(stored, expected);
}

#[test]
fn equal_local_columns_in_different_dimensions_save_concurrently() {
    let metadata = WorldMetadata::with_seed(9);
    let (store, control) = GatedFirstSaveStore::new(metadata, None);
    let repository = ChunkRepository::new(store);
    let position = ChunkPos::ZERO;
    let overworld_address = overworld(position);
    let grass_address = ChunkAddress::new(DimensionId::GRASS_FLOOR, position);
    let (mut app, overworld_owner) = save_app(repository.clone(), 2);
    let mut overworld_chunk = Chunk::default();
    overworld_chunk.set_cell_xyz(0, 0, 0, Item::OakLog.into());
    let mut overworld_heightmap = ChunkHeightmap::default();
    overworld_heightmap.heights[0][0] = 3;
    let overworld_entity = spawn_dirty_chunk(
        &mut app,
        overworld_owner,
        position,
        overworld_chunk.clone(),
        overworld_heightmap,
    );

    app.update();
    control.wait_until_first_started();

    let grass_definition = *repository.catalog().get(DimensionId::GRASS_FLOOR).unwrap();
    let grass_owner = spawn_defined_dimension(&mut app, grass_definition, false);
    let mut grass_chunk = Chunk::default();
    grass_chunk.set_cell_xyz(0, 0, 0, Item::Grass.into());
    let mut grass_heightmap = ChunkHeightmap::default();
    grass_heightmap.heights[0][0] = 7;
    let grass_entity = spawn_dirty_chunk(
        &mut app,
        grass_owner,
        position,
        grass_chunk.clone(),
        grass_heightmap,
    );
    app.world_mut()
        .entity_mut(overworld_owner)
        .remove::<Active>();
    app.world_mut().entity_mut(grass_owner).insert(Active);

    app.update();

    let save_tasks = app.world().resource::<ChunkSaveTasks>();
    assert!(save_tasks.in_flight.contains_key(&overworld_address));
    assert!(save_tasks.in_flight.contains_key(&grass_address));

    control.release_first();
    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(overworld_entity).is_none()
            && world.get::<ChunkNeedsSave>(grass_entity).is_none()
    });

    let (stored_overworld, stored_overworld_heightmap) =
        repository.load_chunk(overworld_address).unwrap().unwrap();
    let (stored_grass, stored_grass_heightmap) =
        repository.load_chunk(grass_address).unwrap().unwrap();
    assert_eq!(stored_overworld, overworld_chunk);
    assert_eq!(stored_overworld_heightmap, overworld_heightmap);
    assert_eq!(stored_grass, grass_chunk);
    assert_eq!(stored_grass_heightmap, grass_heightmap);
}

#[test]
fn dirty_chunks_are_persisted_and_marked_clean() {
    let metadata = WorldMetadata::with_seed(9);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata));
    let position = ChunkPos::new(2, 0, -1);
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(0, 0, 0, Item::OakLog.into());
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
fn dirty_chunks_are_persisted_under_their_root_dimension() {
    let metadata = WorldMetadata::with_seed(9);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata));
    let position = ChunkPos::new(2, 0, -1);
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(0, 0, 0, Item::Grass.into());
    let (mut app, owner) =
        save_app_in_dimension(repository.clone(), usize::MAX, DimensionId::GRASS_FLOOR);
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

    let address = ChunkAddress::new(DimensionId::GRASS_FLOOR, position);
    let (loaded, _) = repository.load_chunk(address).unwrap().unwrap();
    assert_eq!(loaded, chunk);
    assert!(
        repository
            .load_chunk(overworld(position))
            .unwrap()
            .is_none()
    );
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

    let save_tasks = app.world().resource::<ChunkSaveTasks>();
    assert_eq!(save_tasks.in_flight.len(), 1);
    assert_eq!(save_tasks.pending.len(), 1);
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

    assert_eq!(app.world().resource::<ChunkSaveTasks>().in_flight.len(), 1);
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
    original.set_cell_xyz(0, 0, 0, Item::OakLog.into());
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
        .in_flight
        .values()
        .next()
        .unwrap()
        .snapshot
        .source
        .unwrap()
        .revision;
    {
        let mut chunk = app.world_mut().get_mut::<Chunk>(entity).unwrap();
        chunk.set_cell_xyz(0, 0, 0, Item::Stone.into());
        chunk.set_cell_xyz(0, 0, 0, Item::OakLog.into());
        assert_eq!(*chunk, original);
        assert_ne!(chunk.content_revision(), first_revision);
    }
    let live_revision = app.world().get::<Chunk>(entity).unwrap().content_revision();

    update_until(&mut app, |world| {
        world.get::<ChunkNeedsSave>(entity).is_some()
            && world
                .resource::<ChunkSaveTasks>()
                .in_flight
                .values()
                .any(|task| {
                    task.snapshot
                        .source
                        .is_some_and(|source| source.revision == live_revision)
                })
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
                .in_flight
                .values()
                .any(|task| {
                    task.snapshot
                        .source
                        .is_some_and(|source| source.revision == revision)
                        && task.snapshot.payload.heightmap == current_heightmap
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
    assert_eq!(save_tasks.in_flight.len(), 1);
    let source = save_tasks
        .in_flight
        .values()
        .next()
        .unwrap()
        .snapshot
        .source
        .unwrap();
    assert_eq!(source.owner, active_owner);
    assert_eq!(source.entity, active);
    assert_ne!(source.entity, inactive);
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
        world.resource::<ChunkSaveTasks>().in_flight.is_empty()
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
fn permanent_failure_retains_the_newest_owned_snapshot_after_despawn() {
    let metadata = WorldMetadata::with_seed(9);
    let (store, control) = GatedFirstSaveStore::new(metadata, Some(ErrorKind::PermissionDenied));
    let repository = ChunkRepository::new(store);
    let address = overworld(ChunkPos::ZERO);
    let (mut app, owner) = save_app(repository.clone(), usize::MAX);
    let mut original = Chunk::default();
    original.set_cell_xyz(0, 0, 0, Item::OakLog.into());
    let entity = spawn_dirty_chunk(
        &mut app,
        owner,
        ChunkPos::ZERO,
        original,
        ChunkHeightmap::default(),
    );

    app.update();
    control.wait_until_first_started();
    app.world_mut()
        .get_mut::<Chunk>(entity)
        .unwrap()
        .set_cell_xyz(1, 0, 0, Item::Stone.into());
    let expected = app.world().get::<Chunk>(entity).unwrap().clone();
    let expected_revision = expected.content_revision();
    app.update();
    control.release_first();

    update_until(&mut app, |world| {
        world
            .resource::<ChunkSaveTasks>()
            .failures
            .get(&address)
            .is_some_and(|failure| failure.retry_after_updates.is_none())
    });

    let save_tasks = app.world().resource::<ChunkSaveTasks>();
    assert_eq!(
        save_tasks.pending[&address]
            .snapshot
            .source
            .unwrap()
            .revision,
        expected_revision
    );
    assert_eq!(
        save_tasks.pending[&address].snapshot.payload.chunk,
        expected
    );

    app.world_mut().despawn(entity);
    app.world_mut().despawn(owner);
    for _ in 0..INITIAL_SAVE_RETRY_DELAY_UPDATES * 2 {
        app.update();
    }

    let save_tasks = app.world().resource::<ChunkSaveTasks>();
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    assert!(save_tasks.pending.contains_key(&address));
    assert!(save_tasks.in_flight.is_empty());
    assert!(save_tasks.failures.contains_key(&address));
    assert!(save_tasks.has_uncommitted_dimension(DimensionId::OVERWORLD));

    let retained = save_tasks.pending[&address].snapshot.payload.clone();
    assert!(
        app.world_mut()
            .resource_mut::<ChunkSaveTasks>()
            .retry_permanent_failure(address)
    );
    assert_eq!(
        app.world().resource::<ChunkSaveTasks>().pending[&address]
            .snapshot
            .payload,
        retained
    );

    update_until(&mut app, |world| {
        !world
            .resource::<ChunkSaveTasks>()
            .has_uncommitted_dimension(DimensionId::OVERWORLD)
    });
    assert_eq!(control.calls.load(Ordering::SeqCst), 2);
    let (stored, _) = repository.load_chunk(address).unwrap().unwrap();
    assert_eq!(stored, expected);
}

#[test]
fn selection_is_round_robin_within_each_priority() {
    let addresses = [
        overworld(ChunkPos::new(0, 0, 0)),
        overworld(ChunkPos::new(1, 0, 0)),
        overworld(ChunkPos::new(2, 0, 0)),
    ];
    let candidates = vec![
        candidate(addresses[2], false),
        candidate(addresses[0], false),
        candidate(addresses[1], false),
    ];
    let mut cursor = None;

    for expected in [addresses[0], addresses[1], addresses[2], addresses[0]] {
        let selected = ordered_candidates(candidates.clone(), cursor)[0].address;
        assert_eq!(selected, expected);
        cursor = Some(selected);
    }
}

#[test]
fn chunks_waiting_to_evict_are_saved_before_resident_chunks() {
    let metadata = WorldMetadata::with_seed(9);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));
    let (mut app, owner) = save_app(repository, 1);
    app.world_mut()
        .get_mut::<DesiredColumnView>(owner)
        .unwrap()
        .refresh(
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

    let address = *app
        .world()
        .resource::<ChunkSaveTasks>()
        .in_flight
        .keys()
        .next()
        .unwrap();
    assert_eq!(address.position(), evicting_position);
}
