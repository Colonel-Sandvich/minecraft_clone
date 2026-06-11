use bevy::{
    platform::collections::HashMap,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};

use crate::world::{
    chunk::{Chunk, ChunkNeedsSave, ChunkPosition},
    generation::WorldMetadata,
    storage::{ChunkRepository, ChunkStoreResult},
};

const INITIAL_SAVE_RETRY_DELAY_UPDATES: u32 = 60;
const MAX_SAVE_RETRY_DELAY_UPDATES: u32 = 600;

#[derive(Resource, Default)]
pub(crate) struct ChunkSaveTasks {
    tasks: HashMap<Entity, Task<ChunkSaveOutput>>,
    retry_after_updates: HashMap<Entity, u32>,
}

impl ChunkSaveTasks {
    fn tick_retry_backoffs(&mut self) {
        for delay in self.retry_after_updates.values_mut() {
            *delay = delay.saturating_sub(1);
        }
    }

    fn can_start(&self, entity: Entity) -> bool {
        !self.tasks.contains_key(&entity)
            && self
                .retry_after_updates
                .get(&entity)
                .is_none_or(|delay| *delay == 0)
    }

    fn record_success(&mut self, entity: Entity) {
        self.retry_after_updates.remove(&entity);
    }

    fn record_failure(&mut self, entity: Entity) {
        self.retry_after_updates
            .entry(entity)
            .and_modify(|delay| *delay = (*delay * 2).min(MAX_SAVE_RETRY_DELAY_UPDATES))
            .or_insert(INITIAL_SAVE_RETRY_DELAY_UPDATES);
    }
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct ChunkSaveBudget(pub usize);

impl Default for ChunkSaveBudget {
    fn default() -> Self {
        Self(2)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChunkSaveRequest {
    entity: Entity,
    pos: IVec3,
    chunk: Chunk,
    metadata: WorldMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChunkSaveOutput {
    entity: Entity,
    pos: IVec3,
    saved_chunk: Chunk,
    result: ChunkStoreResult<()>,
}

pub(crate) fn finish_chunk_save_tasks(
    mut commands: Commands,
    mut save_tasks: ResMut<ChunkSaveTasks>,
    chunks: Query<(&Chunk, Option<&ChunkNeedsSave>)>,
) {
    let mut completed = Vec::new();
    for (entity, task) in save_tasks.tasks.iter_mut() {
        if let Some(output) = check_ready(task) {
            completed.push((*entity, output));
        }
    }

    for (entity, output) in completed {
        save_tasks.tasks.remove(&entity);

        if output.entity != entity {
            warn!(expected = ?entity, actual = ?output.entity, "Chunk save task returned unexpected entity");
            continue;
        }

        match output.result {
            Ok(()) => {
                let Ok((chunk, Some(_))) = chunks.get(entity) else {
                    save_tasks.record_success(entity);
                    continue;
                };

                if *chunk == output.saved_chunk {
                    commands.entity(entity).remove::<ChunkNeedsSave>();
                    save_tasks.record_success(entity);
                }
            }
            Err(error) => {
                warn!(%error, pos = ?output.pos, "Failed to persist dirty chunk");
                save_tasks.record_failure(entity);
            }
        }
    }
}

pub(crate) fn start_chunk_save_tasks(
    dirty_chunks: Query<(Entity, &Chunk, &ChunkPosition), With<ChunkNeedsSave>>,
    repository: Res<ChunkRepository>,
    metadata: Res<WorldMetadata>,
    save_budget: Res<ChunkSaveBudget>,
    mut save_tasks: ResMut<ChunkSaveTasks>,
) {
    if save_budget.0 == 0 {
        return;
    }

    save_tasks.tick_retry_backoffs();

    let available_slots = save_budget.0.saturating_sub(save_tasks.tasks.len());
    if available_slots == 0 {
        return;
    }

    let thread_pool = AsyncComputeTaskPool::get();
    let mut started = 0;
    for (entity, chunk, position) in dirty_chunks.iter() {
        if started >= available_slots {
            break;
        }
        if !save_tasks.can_start(entity) {
            continue;
        }

        let request = ChunkSaveRequest {
            entity,
            pos: position.0,
            chunk: chunk.clone(),
            metadata: metadata.clone(),
        };
        let repository = repository.clone();
        let task = thread_pool.spawn(async move { save_chunk_snapshot(request, repository) });
        save_tasks.retry_after_updates.remove(&entity);
        save_tasks.tasks.insert(entity, task);
        started += 1;
    }
}

fn save_chunk_snapshot(request: ChunkSaveRequest, repository: ChunkRepository) -> ChunkSaveOutput {
    let result = repository.save_chunk(request.pos, &request.chunk, &request.metadata);

    ChunkSaveOutput {
        entity: request.entity,
        pos: request.pos,
        saved_chunk: request.chunk,
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block::BlockType,
        world::storage::{ChunkRepository, ChunkStore, ChunkStoreError},
    };

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

    fn add_chunk_save_systems(app: &mut App) {
        app.insert_resource(ChunkSaveTasks::default()).add_systems(
            Update,
            (finish_chunk_save_tasks, start_chunk_save_tasks).chain(),
        );
    }

    #[derive(Default)]
    struct FailingSaveStore;

    impl ChunkStore for FailingSaveStore {
        fn load_chunk(
            &self,
            _pos: IVec3,
            _metadata: &WorldMetadata,
        ) -> ChunkStoreResult<Option<Chunk>> {
            Ok(None)
        }

        fn save_chunk(
            &self,
            _pos: IVec3,
            _chunk: &Chunk,
            _metadata: &WorldMetadata,
        ) -> ChunkStoreResult<()> {
            Err(ChunkStoreError::Io {
                kind: std::io::ErrorKind::Other,
                message: "intentional save failure".to_owned(),
            })
        }
    }

    #[test]
    fn dirty_chunks_are_persisted_and_marked_clean() {
        let metadata = WorldMetadata::with_seed(9);
        let repository = ChunkRepository::default();
        let pos = ivec3(2, 0, -1);
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::OakLog;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata.clone())
            .insert_resource(repository.clone())
            .insert_resource(ChunkSaveBudget(usize::MAX));
        add_chunk_save_systems(&mut app);

        let chunk_entity = app
            .world_mut()
            .spawn((ChunkPosition(pos), chunk.clone(), ChunkNeedsSave))
            .id();

        update_until(&mut app, |world| {
            world.get::<ChunkNeedsSave>(chunk_entity).is_none()
        });

        assert_eq!(repository.load_chunk(pos, &metadata).unwrap(), Some(chunk));
    }

    #[test]
    fn save_budget_limits_in_flight_chunks() {
        let metadata = WorldMetadata::with_seed(9);
        let repository = ChunkRepository::default();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata)
            .insert_resource(repository)
            .insert_resource(ChunkSaveBudget(1));
        add_chunk_save_systems(&mut app);

        app.world_mut()
            .spawn((ChunkPosition(IVec3::ZERO), Chunk::default(), ChunkNeedsSave));
        app.world_mut().spawn((
            ChunkPosition(ivec3(1, 0, 0)),
            Chunk::default(),
            ChunkNeedsSave,
        ));

        app.update();

        assert_eq!(app.world().resource::<ChunkSaveTasks>().tasks.len(), 1);
    }

    #[test]
    fn changed_chunks_stay_dirty_until_latest_snapshot_is_saved() {
        let metadata = WorldMetadata::with_seed(9);
        let repository = ChunkRepository::default();
        let pos = ivec3(2, 0, -1);
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::OakLog;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata.clone())
            .insert_resource(repository.clone())
            .insert_resource(ChunkSaveBudget(usize::MAX));
        add_chunk_save_systems(&mut app);

        let chunk_entity = app
            .world_mut()
            .spawn((ChunkPosition(pos), chunk, ChunkNeedsSave))
            .id();

        app.update();
        app.world_mut()
            .get_mut::<Chunk>(chunk_entity)
            .unwrap()
            .blocks[0][0][0] = BlockType::Stone;

        update_until(&mut app, |world| {
            world.get::<ChunkNeedsSave>(chunk_entity).is_none()
        });

        let mut expected = Chunk::default();
        expected.blocks[0][0][0] = BlockType::Stone;
        assert_eq!(
            repository.load_chunk(pos, &metadata).unwrap(),
            Some(expected)
        );
    }

    #[test]
    fn failed_saves_back_off_before_retrying() {
        let metadata = WorldMetadata::with_seed(9);
        let repository = ChunkRepository::new(FailingSaveStore);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(metadata)
            .insert_resource(repository)
            .insert_resource(ChunkSaveBudget(usize::MAX));
        add_chunk_save_systems(&mut app);

        let chunk_entity = app
            .world_mut()
            .spawn((ChunkPosition(IVec3::ZERO), Chunk::default(), ChunkNeedsSave))
            .id();

        update_until(&mut app, |world| {
            world
                .resource::<ChunkSaveTasks>()
                .retry_after_updates
                .contains_key(&chunk_entity)
        });

        app.update();

        let save_tasks = app.world().resource::<ChunkSaveTasks>();
        assert!(!save_tasks.tasks.contains_key(&chunk_entity));
        assert!(save_tasks.retry_after_updates[&chunk_entity] < INITIAL_SAVE_RETRY_DELAY_UPDATES);
    }
}
