mod fluid;
mod invalidation;
mod lifecycle;
mod light;
mod persistence;
mod tasks;
mod view;

use bevy::{
    platform::collections::HashMap,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task},
};

#[cfg(test)]
use bevy::tasks::{TaskPool, TaskPoolBuilder};

use crate::game_state::{GameState, Playing};
use core::future::Future;

use self::{
    fluid::DimensionFluidPlugin,
    lifecycle::{finish_chunk_load_tasks, maintain_chunk_view, start_chunk_load_tasks},
    light::rebuild_chunk_light,
    persistence::{ChunkSaveBudget, finish_chunk_save_tasks, start_chunk_save_tasks},
};
use super::{chunk::ChunkPos, generation::WorldMetadata, storage::ChunkRepository};

pub(crate) use self::{persistence::ChunkSaveTasks, tasks::ChunkLoadTasks};
pub use self::{
    tasks::{ChunkLoadBudget, ChunkSpawnBudget},
    view::{ViewDistance, chunk_positions_in_view},
};
pub(crate) use invalidation::apply_chunk_invalidations;

#[derive(Resource)]
pub(crate) struct ChunkTaskPool(ChunkTaskPoolInner);

enum ChunkTaskPoolInner {
    Global,
    #[cfg(test)]
    Test(TaskPool),
}

impl ChunkTaskPool {
    pub(crate) fn global() -> Self {
        Self(ChunkTaskPoolInner::Global)
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self(ChunkTaskPoolInner::Test(
            TaskPoolBuilder::new().num_threads(1).build(),
        ))
    }

    pub(crate) fn spawn<T: Send + 'static>(
        &self,
        future: impl Future<Output = T> + Send + 'static,
    ) -> Task<T> {
        match &self.0 {
            ChunkTaskPoolInner::Global => AsyncComputeTaskPool::get().spawn(future),
            #[cfg(test)]
            ChunkTaskPoolInner::Test(pool) => pool.spawn(future),
        }
    }
}

#[derive(Default, Component)]
pub struct Dimension {
    chunks: HashMap<ChunkPos, Entity>,
}

impl Dimension {
    pub fn chunk_entity(&self, pos: impl Into<ChunkPos>) -> Option<Entity> {
        self.chunks.get(&pos.into()).copied()
    }

    pub fn contains_chunk(&self, pos: impl Into<ChunkPos>) -> bool {
        self.chunks.contains_key(&pos.into())
    }

    pub fn register_chunk(&mut self, pos: impl Into<ChunkPos>, entity: Entity) -> Option<Entity> {
        self.chunks.insert(pos.into(), entity)
    }

    pub fn unregister_chunk(&mut self, pos: impl Into<ChunkPos>) -> Option<Entity> {
        self.chunks.remove(&pos.into())
    }

    pub fn iter_chunks(&self) -> impl ExactSizeIterator<Item = (ChunkPos, Entity)> + '_ {
        self.chunks.iter().map(|(&pos, &entity)| (pos, entity))
    }

    pub fn chunk_entities(&self) -> &HashMap<ChunkPos, Entity> {
        &self.chunks
    }

    pub fn loaded_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn chunk_map_capacity(&self) -> usize {
        self.chunks.capacity()
    }
}

pub struct DimensionPlugin;

impl Plugin for DimensionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChunkTaskPool::global())
            .init_resource::<WorldMetadata>()
            .init_resource::<ChunkRepository>()
            .init_resource::<ChunkLoadBudget>()
            .init_resource::<ChunkSpawnBudget>()
            .init_resource::<ChunkSaveBudget>()
            .init_resource::<ChunkSaveTasks>()
            .init_resource::<ChunkLoadTasks>()
            .init_resource::<ViewDistance>()
            .add_plugins(DimensionFluidPlugin);

        app.add_systems(
            OnEnter(GameState::GenWorld),
            (
                setup,
                maintain_chunk_view,
                start_chunk_load_tasks,
                finish_chunk_load_tasks,
                |mut game_state: ResMut<NextState<GameState>>| game_state.set(GameState::Playing),
            )
                .chain(),
        );

        app.add_systems(
            Update,
            (
                maintain_chunk_view,
                start_chunk_load_tasks,
                finish_chunk_load_tasks,
                rebuild_chunk_light,
            )
                .chain()
                .in_set(Playing),
        );
        app.add_systems(
            PostUpdate,
            (finish_chunk_save_tasks, start_chunk_save_tasks)
                .chain()
                .run_if(in_state(GameState::Playing)),
        );
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Dimension::default(),
        Transform::default(),
        Visibility::default(),
        Active,
    ));
}

#[derive(Component)]
pub struct Active;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_registry_retains_typed_chunk_positions() {
        let position = ChunkPos::new(-5, 2, 9);
        let entity = Entity::PLACEHOLDER;
        let mut dimension = Dimension::default();

        assert_eq!(dimension.register_chunk(position, entity), None);
        assert_eq!(dimension.chunk_entity(position), Some(entity));
        assert_eq!(dimension.chunk_entity(position.as_ivec3()), Some(entity));
        assert!(dimension.chunk_entities().contains_key(&position));
        assert_eq!(
            dimension.iter_chunks().collect::<Vec<_>>(),
            vec![(position, entity)]
        );
        assert_eq!(
            dimension.unregister_chunk(position.as_ivec3()),
            Some(entity)
        );
        assert!(!dimension.contains_chunk(position));
    }
}
