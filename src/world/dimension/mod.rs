mod fluid;
mod invalidation;
mod light;
mod light_patch;
mod light_task;
mod persistence;
mod streaming;
mod view;

use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task},
};

#[cfg(test)]
use bevy::tasks::{TaskPool, TaskPoolBuilder};

use crate::game_state::{GameState, Playing};
use core::future::Future;

use self::{
    fluid::DimensionFluidPlugin,
    light::{cancel_inactive_dimension_light_tasks, rebuild_chunk_light},
    light_task::DimensionLightTasks,
    persistence::{ChunkSaveBudget, finish_chunk_save_tasks, start_chunk_save_tasks},
    streaming::{
        ColumnExposure, ColumnLightRevision, ColumnLighting, DimensionStreamState,
        LightPatchTicket, ResidentColumnState, finish_column_loads, maintain_column_residency,
        publish_lit_columns, refresh_desired_column_view, start_column_loads,
    },
};
use super::{
    chunk::{ChunkColumn, ChunkPos},
    generation::{WorldHeight, WorldMetadata},
    storage::ChunkRepository,
};

pub(crate) use self::persistence::ChunkSaveTasks;
pub use self::{
    light_patch::ColumnLightBudget,
    streaming::{ColumnActivationBudget, ColumnLoadBudget, ColumnStagingBudget},
    view::{DesiredColumnView, ViewDistance},
};
pub(crate) use invalidation::apply_chunk_invalidations;
pub(crate) use streaming::{ColumnEvictionTicket, ColumnLoadTaskStats, ColumnLoadTicket};

/// Update-phase boundary after dimension streaming and publication complete.
///
/// Visual consumers run after this set so publication markers can be consumed
/// before the frame is extracted for rendering.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DimensionStreamingSet;

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
    loaded_chunks: HashMap<ChunkPos, Entity>,
    published_chunks: HashSet<ChunkPos>,
    loaded_columns: HashMap<ChunkColumn, LoadedColumnHandle>,
    height: WorldHeight,
    stream: DimensionStreamState,
    light_tasks: DimensionLightTasks,
}

#[derive(Debug)]
struct LoadedColumnHandle {
    incarnation: Entity,
}

pub(crate) struct EvictedColumn {
    pub(crate) incarnation: Entity,
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
            loaded_chunks: HashMap::default(),
            published_chunks: HashSet::default(),
            loaded_columns: HashMap::default(),
            height,
            stream: DimensionStreamState::new(owner),
            light_tasks: DimensionLightTasks::default(),
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

    pub fn loaded_chunk_entity(&self, position: ChunkPos) -> Option<Entity> {
        self.loaded_chunks.get(&position).copied()
    }

    pub fn published_chunk_entity(&self, position: ChunkPos) -> Option<Entity> {
        self.published_chunks
            .contains(&position)
            .then(|| self.loaded_chunk_entity(position))
            .flatten()
    }

    pub fn contains_loaded_chunk(&self, position: ChunkPos) -> bool {
        self.loaded_chunks.contains_key(&position)
    }

    pub fn contains_published_chunk(&self, position: ChunkPos) -> bool {
        self.published_chunks.contains(&position)
    }

    /// Registers an independently constructed chunk as both loaded and
    /// published. Streaming uses the full-column transition methods instead.
    pub fn register_published_chunk(
        &mut self,
        position: ChunkPos,
        entity: Entity,
    ) -> Option<Entity> {
        assert!(
            self.height.contains_chunk(position),
            "registered chunk must be within the dimension height"
        );
        assert!(
            !self.loaded_columns.contains_key(&position.column()),
            "independent chunk registration cannot alter a streamed column"
        );
        let previous = self.loaded_chunks.get(&position).copied();
        let previous_published = self.published_chunks.contains(&position);
        assert_eq!(
            previous.is_some(),
            previous_published,
            "loaded and published test registrations must remain aligned"
        );
        self.loaded_chunks.insert(position, entity);
        self.published_chunks.insert(position);
        previous
    }

    pub fn unregister_published_chunk(&mut self, position: ChunkPos) -> Option<Entity> {
        assert!(
            self.height.contains_chunk(position),
            "unregistered chunk must be within the dimension height"
        );
        assert!(
            !self.loaded_columns.contains_key(&position.column()),
            "independent chunk unregistration cannot alter a streamed column"
        );
        let loaded = self.loaded_chunks.get(&position).copied();
        let published = self.published_chunks.contains(&position);
        assert_eq!(
            loaded.is_some(),
            published,
            "loaded and published test registrations must remain aligned"
        );
        self.loaded_chunks.remove(&position);
        self.published_chunks.remove(&position);
        loaded
    }

    pub fn iter_loaded_chunks(&self) -> impl ExactSizeIterator<Item = (ChunkPos, Entity)> + '_ {
        self.loaded_chunks
            .iter()
            .map(|(&position, &entity)| (position, entity))
    }

    pub fn iter_published_chunks(&self) -> impl ExactSizeIterator<Item = (ChunkPos, Entity)> + '_ {
        self.published_chunks.iter().map(|&position| {
            let entity = self
                .loaded_chunk_entity(position)
                .expect("published chunk must remain loaded");
            (position, entity)
        })
    }

    pub fn loaded_chunk_entities(&self) -> &HashMap<ChunkPos, Entity> {
        &self.loaded_chunks
    }

    pub fn loaded_chunk_count(&self) -> usize {
        self.loaded_chunks.len()
    }

    pub fn published_chunk_count(&self) -> usize {
        self.published_chunks.len()
    }

    pub fn chunk_registry_capacity(&self) -> usize {
        self.loaded_chunks.capacity() + self.published_chunks.capacity()
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

    pub(crate) const fn light_tasks(&self) -> &DimensionLightTasks {
        &self.light_tasks
    }

    pub(crate) fn light_tasks_mut(&mut self) -> &mut DimensionLightTasks {
        &mut self.light_tasks
    }

    pub(crate) fn assert_stream_owner(&self, owner: Entity) {
        assert_eq!(
            self.stream.owner(),
            owner,
            "dimension streaming state must remain bound to its entity"
        );
    }

    pub(crate) fn has_any_loaded_chunk_in_column(&self, column: ChunkColumn) -> bool {
        (0..self.height.chunks_i32()).any(|y| self.contains_loaded_chunk(column.chunk(y)))
    }

    pub(crate) fn has_complete_loaded_column(&self, column: ChunkColumn) -> bool {
        (0..self.height.chunks_i32()).all(|y| self.contains_loaded_chunk(column.chunk(y)))
    }

    pub(crate) fn has_complete_resident_light_neighborhood(&self, column: ChunkColumn) -> bool {
        column.chebyshev_neighborhood(1).all(|dependency| {
            self.has_complete_loaded_column(dependency)
                && self.resident_column_state(dependency).is_some()
        })
    }

    pub(crate) fn complete_loaded_column(
        &self,
        column: ChunkColumn,
    ) -> Option<Vec<(ChunkPos, Entity)>> {
        (0..self.height.chunks_i32())
            .map(|y| {
                let position = column.chunk(y);
                self.loaded_chunk_entity(position)
                    .map(|entity| (position, entity))
            })
            .collect()
    }

    pub(crate) fn install_accepted_column(
        &mut self,
        ticket: ColumnLoadTicket,
        incarnation: Entity,
        entities: Vec<Entity>,
    ) {
        assert_eq!(entities.len(), self.height.chunks());
        assert_eq!(ticket.owner(), self.stream.owner());
        assert!(
            (0..self.height.chunks_i32())
                .all(|y| !self.contains_loaded_chunk(ticket.column().chunk(y))),
            "loaded column must not overlap registered chunks"
        );
        assert!(
            !self.loaded_columns.contains_key(&ticket.column()),
            "loaded column incarnation must be unique"
        );
        assert!(
            self.stream.activate_load(ticket),
            "accepted column load must still be current when installed"
        );

        for (y, &entity) in entities.iter().enumerate() {
            let previous = self
                .loaded_chunks
                .insert(ticket.column().chunk(y as i32), entity);
            debug_assert!(previous.is_none());
        }
        self.loaded_columns
            .insert(ticket.column(), LoadedColumnHandle { incarnation });
    }

    fn expose_loaded_column(&mut self, column: ChunkColumn) -> bool {
        if !self.loaded_columns.contains_key(&column)
            || !self.has_complete_loaded_column(column)
            || (0..self.height.chunks_i32())
                .any(|y| self.published_chunks.contains(&column.chunk(y)))
        {
            return false;
        }
        let chunks = self
            .complete_loaded_column(column)
            .expect("loaded column must contain every configured Y chunk");
        for (position, _) in chunks {
            self.published_chunks.insert(position);
        }
        true
    }

    fn hide_loaded_column(&mut self, column: ChunkColumn) -> bool {
        if !self.loaded_columns.contains_key(&column)
            || !(0..self.height.chunks_i32())
                .all(|y| self.published_chunks.contains(&column.chunk(y)))
        {
            return false;
        }
        for y in 0..self.height.chunks_i32() {
            assert!(self.published_chunks.remove(&column.chunk(y)));
        }
        true
    }

    pub(crate) fn resident_column_state(&self, column: ChunkColumn) -> Option<ResidentColumnState> {
        self.stream.resident_state(column)
    }

    pub(crate) fn column_lighting(&self, column: ChunkColumn) -> Option<ColumnLighting> {
        self.stream.column_lighting(column)
    }

    pub(crate) fn column_exposure(&self, column: ChunkColumn) -> Option<ColumnExposure> {
        self.stream.column_exposure(column)
    }

    pub(crate) fn mark_column_light_pending(&mut self, column: ChunkColumn) -> bool {
        let active = self.stream.light_patch_ticket(column);
        let changed = self.stream.mark_light_pending(column);
        if let Some(ticket) = active {
            self.light_tasks.cancel(ticket);
        }
        changed
    }

    pub(crate) fn begin_column_light_patch(
        &mut self,
        commit_columns: &[ChunkColumn],
    ) -> Option<LightPatchTicket> {
        self.stream.begin_light_patch(commit_columns)
    }

    pub(crate) fn finish_column_light_patch(
        &mut self,
        ticket: LightPatchTicket,
    ) -> Option<Vec<(ChunkColumn, ColumnLightRevision)>> {
        self.stream.finish_light_patch(ticket)
    }

    pub(crate) fn cancel_column_light_patch(&mut self, ticket: LightPatchTicket) -> bool {
        let task_cancelled = self.light_tasks.cancel(ticket);
        let claim_cancelled = self.stream.cancel_light_patch(ticket);
        task_cancelled || claim_cancelled
    }

    pub(crate) fn cancel_light_task_depending_on(&mut self, column: ChunkColumn) -> bool {
        if !self.light_tasks.active_depends_on(column) {
            return false;
        }
        let ticket = self
            .light_tasks
            .active_ticket()
            .expect("dependent light task must have an active ticket");
        self.cancel_column_light_patch(ticket)
    }

    pub(crate) fn publish_lit_column(&mut self, column: ChunkColumn) -> bool {
        if !self.loaded_columns.contains_key(&column)
            || !self.has_complete_loaded_column(column)
            || (0..self.height.chunks_i32())
                .any(|y| self.published_chunks.contains(&column.chunk(y)))
            || !self.stream.publish(column)
        {
            return false;
        }
        assert!(
            self.expose_loaded_column(column),
            "published resident column must remain loaded"
        );
        true
    }

    pub(crate) fn unpublish_column(&mut self, column: ChunkColumn) -> bool {
        if !self.loaded_columns.contains_key(&column)
            || !(0..self.height.chunks_i32())
                .all(|y| self.published_chunks.contains(&column.chunk(y)))
            || !self.stream.unpublish(column)
        {
            return false;
        }
        assert!(
            self.hide_loaded_column(column),
            "unpublished resident column must have been exposed"
        );
        true
    }

    pub(crate) fn column_incarnation(&self, column: ChunkColumn) -> Option<Entity> {
        self.loaded_columns
            .get(&column)
            .map(|handle| handle.incarnation)
    }

    pub(crate) fn evict_column(&mut self, ticket: ColumnEvictionTicket) -> Option<EvictedColumn> {
        assert_eq!(ticket.owner(), self.stream.owner());
        let chunks = self.complete_loaded_column(ticket.column())?;
        let incarnation = self.loaded_columns.get(&ticket.column())?.incarnation;
        assert!(
            chunks
                .iter()
                .all(|(position, _)| !self.published_chunks.contains(position)),
            "evicted column must be unpublished before registry removal"
        );
        if !self.stream.commit_eviction(ticket) {
            return None;
        }
        for (position, entity) in &chunks {
            self.published_chunks.remove(position);
            assert_eq!(self.loaded_chunks.remove(position), Some(*entity));
        }
        let handle = self
            .loaded_columns
            .remove(&ticket.column())
            .expect("resident column must retain its incarnation until eviction");
        assert_eq!(handle.incarnation, incarnation);
        Some(EvictedColumn { incarnation })
    }
}

pub struct DimensionPlugin;

impl Plugin for DimensionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChunkTaskPool::global())
            .init_resource::<WorldMetadata>()
            .init_resource::<ChunkRepository>()
            .init_resource::<ColumnLoadBudget>()
            .init_resource::<ColumnStagingBudget>()
            .init_resource::<ColumnActivationBudget>()
            .init_resource::<ColumnLightBudget>()
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
                cancel_inactive_dimension_light_tasks,
                refresh_desired_column_view,
                maintain_column_residency,
                finish_column_loads,
                rebuild_chunk_light,
                publish_lit_columns,
                start_column_loads,
            )
                .chain()
                .in_set(Playing)
                .in_set(DimensionStreamingSet),
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

        assert_eq!(dimension.register_published_chunk(position, entity), None);
        assert_eq!(dimension.loaded_chunk_entity(position), Some(entity));
        assert_eq!(dimension.published_chunk_entity(position), Some(entity));
        assert_eq!(
            dimension.iter_loaded_chunks().collect::<Vec<_>>(),
            vec![(position, entity)]
        );
        assert_eq!(
            dimension.iter_published_chunks().collect::<Vec<_>>(),
            vec![(position, entity)]
        );
        assert_eq!(dimension.unregister_published_chunk(position), Some(entity));
        assert!(!dimension.contains_loaded_chunk(position));
        assert!(!dimension.contains_published_chunk(position));
    }

    #[test]
    #[should_panic(expected = "registered chunk must be within the dimension height")]
    fn dimension_registry_rejects_out_of_range_chunk_positions() {
        let mut dimension = Dimension::default();

        dimension.register_published_chunk(
            ChunkPos::new(0, dimension.height().chunks_i32(), 0),
            Entity::PLACEHOLDER,
        );
    }

    #[test]
    fn streamed_column_publication_is_a_view_over_loaded_entities() {
        let height = WorldHeight::new(2).unwrap();
        let column = ChunkColumn::new(-4, 7);
        let entities = [Entity::from_bits(11), Entity::from_bits(12)];
        let mut dimension = Dimension::new(Entity::PLACEHOLDER, height);
        dimension.loaded_columns.insert(
            column,
            LoadedColumnHandle {
                incarnation: Entity::from_bits(10),
            },
        );
        for (y, entity) in entities.into_iter().enumerate() {
            dimension
                .loaded_chunks
                .insert(column.chunk(y as i32), entity);
        }

        assert_eq!(dimension.loaded_chunk_count(), height.chunks());
        assert_eq!(dimension.published_chunk_count(), 0);
        assert!(dimension.expose_loaded_column(column));
        for (y, entity) in entities.into_iter().enumerate() {
            let position = column.chunk(y as i32);
            assert_eq!(dimension.loaded_chunk_entity(position), Some(entity));
            assert_eq!(dimension.published_chunk_entity(position), Some(entity));
        }

        assert!(dimension.hide_loaded_column(column));
        assert_eq!(dimension.loaded_chunk_count(), height.chunks());
        assert_eq!(dimension.published_chunk_count(), 0);
        assert!(!dimension.hide_loaded_column(column));
        for y in 0..height.chunks_i32() {
            let position = column.chunk(y);
            assert!(dimension.contains_loaded_chunk(position));
            assert!(!dimension.contains_published_chunk(position));
        }
    }
}
