use std::ops::Mul;

use bevy::{
    platform::collections::HashSet,
    prelude::*,
    tasks::{AsyncComputeTaskPool, futures::check_ready},
};

use crate::{
    player::Player,
    world::{
        chunk::{
            CHUNK_SIZE, ChunkNeedsColliderRebuild, ChunkNeedsMeshRebuild, ChunkNeedsSave,
            ChunkPosition,
        },
        generation::WorldMetadata,
        loading::{ChunkLoadRequest, load_or_generate_chunk},
        storage::ChunkRepository,
    },
};

use super::{
    Active, Dimension,
    tasks::{ChunkLoadBudget, ChunkLoadTasks, ChunkSpawnBudget},
    view::{ViewDistance, chunk_positions_in_view},
};

pub(crate) fn maintain_chunk_view(
    mut commands: Commands,
    dimension: Single<(&mut Dimension, Entity), With<Active>>,
    maybe_player_q: Option<Single<&Transform, With<Player>>>,
    dirty_chunks: Query<Option<&ChunkNeedsSave>>,
    metadata: Res<WorldMetadata>,
    view_distance: Res<ViewDistance>,
    mut load_tasks: ResMut<ChunkLoadTasks>,
) {
    let centre = maybe_player_q.map_or(Transform::default(), |q| **q);
    let chunks_in_view = chunk_positions_in_view(
        centre.translation,
        metadata.height_chunks,
        view_distance.chunks(),
    );
    let chunks_in_view_set = chunks_in_view.iter().copied().collect::<HashSet<_>>();

    let (mut dim, _) = dimension.into_inner();

    let chunks_to_unload = dim
        .chunks
        .iter()
        .filter(|(pos, _)| !chunks_in_view_set.contains(*pos))
        .map(|(pos, entity)| (*pos, *entity))
        .collect::<Vec<_>>();

    for (pos, entity) in chunks_to_unload {
        if matches!(dirty_chunks.get(entity), Ok(Some(_))) {
            continue;
        }

        dim.chunks.remove(&pos);
        commands.entity(entity).despawn();
    }

    load_tasks.retain_visible(&chunks_in_view_set);
}

pub(crate) fn start_chunk_load_tasks(
    dimension: Single<&Dimension, With<Active>>,
    maybe_player_q: Option<Single<&Transform, With<Player>>>,
    repository: Res<ChunkRepository>,
    metadata: Res<WorldMetadata>,
    view_distance: Res<ViewDistance>,
    load_budget: Res<ChunkLoadBudget>,
    mut load_tasks: ResMut<ChunkLoadTasks>,
) {
    load_tasks.tick_failure_backoffs();

    if load_budget.0 == 0 {
        return;
    }

    let available_slots = load_budget.0.saturating_sub(load_tasks.tasks.len());
    if available_slots == 0 {
        return;
    }

    let centre = maybe_player_q.map_or(Transform::default(), |q| **q);
    let dim = dimension.into_inner();
    let thread_pool = AsyncComputeTaskPool::get();
    let mut started = 0;

    for pos in chunk_positions_in_view(
        centre.translation,
        metadata.height_chunks,
        view_distance.chunks(),
    ) {
        if started >= available_slots {
            break;
        }

        if dim.chunks.contains_key(&pos) || load_tasks.blocks_starting_task(pos) {
            continue;
        }

        let request = ChunkLoadRequest::new(pos);
        let repository = repository.clone();
        let task = thread_pool.spawn(async move { load_or_generate_chunk(request, repository) });
        load_tasks.tasks.insert(pos, task);
        started += 1;
    }
}

pub(crate) fn finish_chunk_load_tasks(
    mut commands: Commands,
    dimension: Single<(&mut Dimension, Entity), With<Active>>,
    maybe_player_q: Option<Single<&Transform, With<Player>>>,
    spawn_budget: Res<ChunkSpawnBudget>,
    mut load_tasks: ResMut<ChunkLoadTasks>,
    metadata: Res<WorldMetadata>,
    view_distance: Res<ViewDistance>,
) {
    if spawn_budget.0 == 0 {
        return;
    }

    let centre = maybe_player_q.map_or(Transform::default(), |q| **q);
    let mut completed = Vec::new();
    for pos in chunk_positions_in_view(
        centre.translation,
        metadata.height_chunks,
        view_distance.chunks(),
    ) {
        if completed.len() >= spawn_budget.0 {
            break;
        }
        let Some(task) = load_tasks.tasks.get_mut(&pos) else {
            continue;
        };
        if let Some(loaded) = check_ready(task) {
            completed.push((pos, loaded));
        }
    }

    let (mut dim, dimension_entity) = dimension.into_inner();
    for (pos, loaded) in completed {
        load_tasks.tasks.remove(&pos);

        if dim.chunks.contains_key(&pos) {
            load_tasks.record_success(pos);
            continue;
        }

        if loaded.pos != pos {
            warn!(expected = ?pos, actual = ?loaded.pos, "Chunk load task returned unexpected position");
            continue;
        }
        let loaded = match loaded.result {
            Ok(loaded) => loaded,
            Err(error) => {
                warn!(%error, ?pos, "Failed to load chunk; leaving it unavailable");
                load_tasks.record_failure(pos, error);
                continue;
            }
        };

        load_tasks.record_success(pos);
        let chunk_entity = commands
            .spawn((
                ChildOf(dimension_entity),
                ChunkPosition(pos),
                ChunkNeedsMeshRebuild,
                ChunkNeedsColliderRebuild,
                loaded.chunk,
                Transform::from_translation(pos.as_vec3().mul(CHUNK_SIZE as f32)),
                Visibility::default(),
            ))
            .id();

        dim.chunks.insert(pos, chunk_entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::platform::collections::HashMap;

    use crate::world::chunk::Chunk;
    use crate::world::storage::{
        ChunkStore, ChunkStoreError, ChunkStoreResult, InMemoryChunkStore,
    };

    const TEST_VIEW_DISTANCE: i32 = 14;

    fn test_metadata() -> WorldMetadata {
        WorldMetadata::with_seed(1)
    }

    fn expected_chunk_count(metadata: &WorldMetadata) -> usize {
        chunk_positions_in_view(Vec3::ZERO, metadata.height_chunks, TEST_VIEW_DISTANCE).len()
    }

    fn add_chunk_lifecycle_systems(app: &mut App) {
        app.insert_resource(ChunkLoadTasks::default())
            .insert_resource(ViewDistance::new(TEST_VIEW_DISTANCE))
            .add_systems(
                Update,
                (
                    maintain_chunk_view,
                    start_chunk_load_tasks,
                    finish_chunk_load_tasks,
                )
                    .chain(),
            );
    }

    fn update_until(app: &mut App, mut predicate: impl FnMut(&World) -> bool) {
        for _ in 0..100 {
            app.update();
            if predicate(app.world()) {
                return;
            }
            std::thread::yield_now();
        }

        panic!("condition was not met after 100 updates");
    }

    #[test]
    fn gen_chunks_in_view_unloads_chunks_outside_view() {
        let metadata = test_metadata();
        let moved_chunk_x = TEST_VIEW_DISTANCE + 2;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata.clone())
            .insert_resource(ChunkRepository::new(InMemoryChunkStore::new(
                metadata.clone(),
            )))
            .insert_resource(ChunkLoadBudget(usize::MAX))
            .insert_resource(ChunkSpawnBudget(usize::MAX));
        add_chunk_lifecycle_systems(&mut app);

        let dimension_entity = app
            .world_mut()
            .spawn((
                Dimension::default(),
                Transform::default(),
                Visibility::default(),
                Active,
            ))
            .id();
        let player = app
            .world_mut()
            .spawn((Player::default(), Transform::default()))
            .id();

        update_until(&mut app, |world| {
            let dimension = world.get::<Dimension>(dimension_entity).unwrap();
            dimension.loaded_chunk_count() == expected_chunk_count(&metadata)
        });

        let origin_chunk = {
            let dimension = app.world().get::<Dimension>(dimension_entity).unwrap();
            assert_eq!(dimension.chunks.len(), expected_chunk_count(&metadata));
            assert_eq!(
                dimension.loaded_chunk_count(),
                expected_chunk_count(&metadata)
            );
            dimension.chunk_entity(IVec3::ZERO).unwrap()
        };

        app.world_mut()
            .entity_mut(player)
            .get_mut::<Transform>()
            .unwrap()
            .translation = Vec3::new(CHUNK_SIZE as f32 * moved_chunk_x as f32, 0.0, 0.0);

        update_until(&mut app, |world| {
            let dimension = world.get::<Dimension>(dimension_entity).unwrap();
            dimension.loaded_chunk_count() == expected_chunk_count(&metadata)
                && dimension.chunk_entity(ivec3(moved_chunk_x, 0, 0)).is_some()
        });

        let dimension = app.world().get::<Dimension>(dimension_entity).unwrap();
        assert_eq!(dimension.chunks.len(), expected_chunk_count(&metadata));
        assert_eq!(
            dimension.loaded_chunk_count(),
            expected_chunk_count(&metadata)
        );
        assert!(dimension.chunk_entity(IVec3::ZERO).is_none());
        assert!(dimension.chunk_entity(ivec3(moved_chunk_x, 0, 0)).is_some());
        assert!(app.world().get_entity(origin_chunk).is_err());
    }

    #[test]
    fn load_budget_limits_in_flight_chunks() {
        let metadata = test_metadata();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata.clone())
            .insert_resource(ChunkRepository::new(InMemoryChunkStore::new(
                metadata.clone(),
            )))
            .insert_resource(ChunkLoadBudget(1))
            .insert_resource(ChunkSpawnBudget(0));
        add_chunk_lifecycle_systems(&mut app);

        let dimension_entity = app
            .world_mut()
            .spawn((
                Dimension::default(),
                Transform::default(),
                Visibility::default(),
                Active,
            ))
            .id();
        app.world_mut()
            .spawn((Player::default(), Transform::default()));

        app.update();

        let dimension = app.world().get::<Dimension>(dimension_entity).unwrap();
        let loading_count = app.world().resource::<ChunkLoadTasks>().tasks.len();

        assert_eq!(dimension.chunks.len(), 0);
        assert_eq!(dimension.loaded_chunk_count(), 0);
        assert_eq!(loading_count, 1);
        assert_eq!(
            expected_chunk_count(&metadata) - loading_count,
            expected_chunk_count(&metadata) - 1
        );
    }

    struct FailingLoadStore {
        metadata: WorldMetadata,
    }

    impl ChunkStore for FailingLoadStore {
        fn metadata(&self) -> &WorldMetadata {
            &self.metadata
        }

        fn load_chunk(&self, _pos: IVec3) -> ChunkStoreResult<Option<Chunk>> {
            Err(ChunkStoreError::WorldMetadataMismatch {
                key: "seed".to_owned(),
                expected: "1".to_owned(),
                found: "2".to_owned(),
            })
        }

        fn save_chunk(&self, _pos: IVec3, _chunk: &Chunk) -> ChunkStoreResult<()> {
            Ok(())
        }
    }

    #[test]
    fn permanent_load_failures_are_not_retried_every_update() {
        let mut metadata = test_metadata();
        metadata.height_chunks = 1;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata.clone())
            .insert_resource(ChunkRepository::new(FailingLoadStore {
                metadata: metadata.clone(),
            }))
            .insert_resource(ChunkLoadBudget(1))
            .insert_resource(ChunkSpawnBudget(usize::MAX));
        add_chunk_lifecycle_systems(&mut app);

        app.world_mut().spawn((
            Dimension::default(),
            Transform::default(),
            Visibility::default(),
            Active,
        ));
        app.world_mut()
            .spawn((Player::default(), Transform::default()));

        update_until(&mut app, |world| {
            world
                .resource::<ChunkLoadTasks>()
                .failures
                .contains_key(&IVec3::ZERO)
        });

        for _ in 0..5 {
            app.update();
        }

        let load_tasks = app.world().resource::<ChunkLoadTasks>();
        let failure = load_tasks.failures.get(&IVec3::ZERO).unwrap();
        assert_eq!(failure.attempts, 1);
        assert_eq!(failure.retry_after_updates, None);
        assert!(!load_tasks.tasks.contains_key(&IVec3::ZERO));
    }

    struct FailingSaveStore {
        metadata: WorldMetadata,
    }

    impl ChunkStore for FailingSaveStore {
        fn metadata(&self) -> &WorldMetadata {
            &self.metadata
        }

        fn load_chunk(&self, _pos: IVec3) -> ChunkStoreResult<Option<Chunk>> {
            Ok(None)
        }

        fn save_chunk(&self, _pos: IVec3, _chunk: &Chunk) -> ChunkStoreResult<()> {
            Err(ChunkStoreError::Io {
                kind: std::io::ErrorKind::Other,
                message: "intentional test failure".to_owned(),
            })
        }
    }

    #[test]
    fn dirty_chunks_stay_loaded_when_unload_save_fails() {
        let metadata = test_metadata();
        let repository = ChunkRepository::new(FailingSaveStore {
            metadata: metadata.clone(),
        });
        let pos = IVec3::ZERO;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata.clone())
            .insert_resource(repository)
            .insert_resource(ChunkLoadTasks::default())
            .insert_resource(ViewDistance::new(TEST_VIEW_DISTANCE))
            .add_systems(Update, maintain_chunk_view);

        let chunk_entity = app
            .world_mut()
            .spawn((ChunkPosition(pos), Chunk::default(), ChunkNeedsSave))
            .id();
        let dimension_entity = app
            .world_mut()
            .spawn((
                Dimension {
                    chunks: HashMap::from([(pos, chunk_entity)]),
                },
                Transform::default(),
                Visibility::default(),
                Active,
            ))
            .id();
        app.world_mut().spawn((
            Player::default(),
            Transform::from_translation(Vec3::new(
                CHUNK_SIZE as f32 * (TEST_VIEW_DISTANCE + 2) as f32,
                0.0,
                0.0,
            )),
        ));

        app.update();

        let dimension = app.world().get::<Dimension>(dimension_entity).unwrap();
        assert_eq!(dimension.chunk_entity(pos), Some(chunk_entity));
        assert!(app.world().get_entity(chunk_entity).is_ok());
    }
}
