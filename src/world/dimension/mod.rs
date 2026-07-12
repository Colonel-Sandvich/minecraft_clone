mod fluid;
mod invalidation;
mod light;
mod persistence;
mod streaming;
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
    light::rebuild_chunk_light,
    persistence::{ChunkSaveBudget, finish_chunk_save_tasks, start_chunk_save_tasks},
    streaming::{
        DimensionStreamState, finish_column_loads, maintain_column_residency,
        refresh_desired_column_view, start_column_loads,
    },
};
use super::{
    chunk::{ChunkColumn, ChunkPos},
    generation::{WorldHeight, WorldMetadata},
    storage::ChunkRepository,
};

pub(crate) use self::persistence::ChunkSaveTasks;
pub use self::{
    streaming::{ColumnActivationBudget, ColumnLoadBudget},
    view::{DesiredColumnView, ViewDistance},
};
pub(crate) use invalidation::apply_chunk_invalidations;
pub(crate) use streaming::{ColumnEvictionTicket, ColumnLoadTaskStats, ColumnLoadTicket};

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

#[derive(Component)]
pub struct Dimension {
    chunks: HashMap<ChunkPos, Entity>,
    height: WorldHeight,
    stream: DimensionStreamState,
}

#[cfg(test)]
impl Default for Dimension {
    fn default() -> Self {
        Self::new(Entity::PLACEHOLDER, WorldHeight::default())
    }
}

impl Dimension {
    pub(crate) fn new(owner: Entity, height: WorldHeight) -> Self {
        Self {
            chunks: HashMap::default(),
            height,
            stream: DimensionStreamState::new(owner),
        }
    }

    pub fn spawn_in_world(world: &mut World, height: WorldHeight) -> Entity {
        let owner = world.spawn_empty().id();
        world.entity_mut(owner).insert(Self::new(owner, height));
        owner
    }

    pub const fn height(&self) -> WorldHeight {
        self.height
    }

    pub fn chunk_entity(&self, pos: impl Into<ChunkPos>) -> Option<Entity> {
        self.chunks.get(&pos.into()).copied()
    }

    pub fn contains_chunk(&self, pos: impl Into<ChunkPos>) -> bool {
        self.chunks.contains_key(&pos.into())
    }

    pub fn register_chunk(&mut self, pos: impl Into<ChunkPos>, entity: Entity) -> Option<Entity> {
        let pos = pos.into();
        assert!(
            self.height.contains_chunk(pos),
            "registered chunk must be within the dimension height"
        );
        self.chunks.insert(pos, entity)
    }

    pub fn unregister_chunk(&mut self, pos: impl Into<ChunkPos>) -> Option<Entity> {
        let pos = pos.into();
        assert!(
            self.height.contains_chunk(pos),
            "unregistered chunk must be within the dimension height"
        );
        self.chunks.remove(&pos)
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

    pub(crate) const fn stream(&self) -> &DimensionStreamState {
        &self.stream
    }

    pub(crate) fn load_task_stats(&self) -> ColumnLoadTaskStats {
        self.stream.stats()
    }

    pub(crate) fn stream_mut(&mut self) -> &mut DimensionStreamState {
        &mut self.stream
    }

    pub(crate) fn assert_stream_owner(&self, owner: Entity) {
        assert_eq!(
            self.stream.owner(),
            owner,
            "dimension streaming state must remain bound to its entity"
        );
    }

    pub(crate) fn has_any_chunk_in_column(&self, column: ChunkColumn) -> bool {
        (0..self.height.chunks_i32()).any(|y| self.contains_chunk(column.chunk(y)))
    }

    pub(crate) fn complete_column(&self, column: ChunkColumn) -> Option<Vec<(ChunkPos, Entity)>> {
        (0..self.height.chunks_i32())
            .map(|y| {
                let position = column.chunk(y);
                self.chunk_entity(position).map(|entity| (position, entity))
            })
            .collect()
    }

    pub(crate) fn publish_accepted_column(
        &mut self,
        ticket: ColumnLoadTicket,
        entities: Vec<Entity>,
    ) {
        assert_eq!(entities.len(), self.height.chunks());
        assert_eq!(ticket.owner(), self.stream.owner());
        assert!(
            (0..self.height.chunks_i32()).all(|y| !self.contains_chunk(ticket.column().chunk(y))),
            "loaded column must not overlap registered chunks"
        );

        for (y, entity) in entities.into_iter().enumerate() {
            self.chunks.insert(ticket.column().chunk(y as i32), entity);
        }
        assert!(
            self.stream.activate_load(ticket),
            "accepted column load must still be current when published"
        );
    }

    pub(crate) fn evict_column(
        &mut self,
        ticket: ColumnEvictionTicket,
    ) -> Option<Vec<(ChunkPos, Entity)>> {
        assert_eq!(ticket.owner(), self.stream.owner());
        let entities = self.complete_column(ticket.column())?;
        if !self.stream.commit_eviction(ticket) {
            return None;
        }
        for (position, _) in &entities {
            self.chunks.remove(position);
        }
        Some(entities)
    }
}

pub struct DimensionPlugin;

impl Plugin for DimensionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChunkTaskPool::global())
            .init_resource::<WorldMetadata>()
            .init_resource::<ChunkRepository>()
            .init_resource::<ColumnLoadBudget>()
            .init_resource::<ColumnActivationBudget>()
            .init_resource::<ChunkSaveBudget>()
            .init_resource::<ChunkSaveTasks>()
            .init_resource::<ViewDistance>()
            .init_resource::<DesiredColumnView>()
            .add_plugins(DimensionFluidPlugin);

        app.add_systems(
            OnEnter(GameState::GenWorld),
            (
                setup,
                refresh_desired_column_view,
                maintain_column_residency,
                finish_column_loads,
                start_column_loads,
                |mut game_state: ResMut<NextState<GameState>>| game_state.set(GameState::Playing),
            )
                .chain(),
        );

        app.add_systems(
            Update,
            (
                refresh_desired_column_view,
                maintain_column_residency,
                finish_column_loads,
                start_column_loads,
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

fn setup(mut commands: Commands, metadata: Res<WorldMetadata>) {
    let entity = commands.spawn_empty().id();
    commands.entity(entity).insert((
        Dimension::new(entity, metadata.height()),
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
    fn immediate_spawn_binds_streaming_state_to_the_allocated_entity() {
        let mut world = World::new();

        let owner = Dimension::spawn_in_world(&mut world, WorldHeight::default());

        world
            .get::<Dimension>(owner)
            .unwrap()
            .assert_stream_owner(owner);
    }

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

    #[test]
    #[should_panic(expected = "registered chunk must be within the dimension height")]
    fn dimension_registry_rejects_out_of_range_chunk_positions() {
        let mut dimension = Dimension::default();

        dimension.register_chunk(
            ChunkPos::new(0, dimension.height().chunks_i32(), 0),
            Entity::PLACEHOLDER,
        );
    }
}
