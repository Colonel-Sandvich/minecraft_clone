#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "queue lifecycle helpers support staged consumer and dimension-switch migrations"
    )
)]
mod derived_work;
mod fluid;
mod invalidation;
mod light;
mod light_patch;
mod light_task;
mod persistence;
mod streaming;
mod switching;
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
    derived_work::{ChunkDerivedEffects, ChunkDerivedWorkKind, DimensionDerivedWork},
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
    definition::{DimensionCatalog, DimensionDefinition, DimensionId},
    generation::WorldHeight,
};

#[cfg(test)]
use super::definition::GeneratorProfile;

pub(crate) use self::derived_work::ChunkDerivedWork;
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
    definition: DimensionDefinition,
    stream: DimensionStreamState,
    light_tasks: DimensionLightTasks,
    derived_work: DimensionDerivedWork,
}

#[derive(Debug)]
struct LoadedColumnHandle {
    incarnation: Entity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EvictedColumn {
    pub(crate) incarnation: Entity,
}

#[cfg(test)]
impl Default for Dimension {
    fn default() -> Self {
        Self::new_for_test(Entity::PLACEHOLDER, WorldHeight::default())
    }
}

impl Dimension {
    pub(crate) fn new(owner: Entity, definition: DimensionDefinition) -> Self {
        Self {
            loaded_chunks: HashMap::default(),
            published_chunks: HashSet::default(),
            loaded_columns: HashMap::default(),
            definition,
            stream: DimensionStreamState::new(owner),
            light_tasks: DimensionLightTasks::default(),
            derived_work: DimensionDerivedWork::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(owner: Entity, height: WorldHeight) -> Self {
        Self::new(
            owner,
            DimensionDefinition::new(
                DimensionId::OVERWORLD,
                height,
                GeneratorProfile::OverworldV1,
                Vec3::ZERO,
            ),
        )
    }

    fn root_components(
        owner: Entity,
        definition: DimensionDefinition,
    ) -> (Self, DesiredColumnView, Transform, Visibility) {
        (
            Self::new(owner, definition),
            DesiredColumnView::default(),
            Transform::default(),
            Visibility::default(),
        )
    }

    pub fn spawn_in_world(
        world: &mut World,
        catalog: &DimensionCatalog,
        id: DimensionId,
    ) -> Entity {
        let definition = *catalog
            .get(id)
            .unwrap_or_else(|| panic!("world catalog does not contain dimension {id}"));
        let owner = world.spawn_empty().id();
        world
            .entity_mut(owner)
            .insert(Self::root_components(owner, definition));
        owner
    }

    pub const fn definition(&self) -> DimensionDefinition {
        self.definition
    }

    pub const fn id(&self) -> DimensionId {
        self.definition.id()
    }

    pub const fn height(&self) -> WorldHeight {
        self.definition.height()
    }

    pub const fn arrival(&self) -> Vec3 {
        self.definition.arrival()
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
            self.height().contains_chunk(position),
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
        if let Some(previous) = previous {
            self.discard_chunk_derived_work(position, previous);
        }
        self.loaded_chunks.insert(position, entity);
        self.published_chunks.insert(position);
        previous
    }

    pub fn unregister_published_chunk(&mut self, position: ChunkPos) -> Option<Entity> {
        assert!(
            self.height().contains_chunk(position),
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
        if let Some(entity) = loaded {
            self.discard_chunk_derived_work(position, entity);
        }
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

    pub fn enqueue_mesh_rebuild(&mut self, position: ChunkPos) -> bool {
        self.enqueue_published_derived_work(position, ChunkDerivedWorkKind::MeshRebuild)
    }

    pub fn enqueue_render_light_upload(&mut self, position: ChunkPos) -> bool {
        self.enqueue_published_derived_work(position, ChunkDerivedWorkKind::RenderLightUpload)
    }

    pub(crate) fn enqueue_collider_rebuild(&mut self, position: ChunkPos) -> bool {
        self.enqueue_published_derived_work(position, ChunkDerivedWorkKind::ColliderRebuild)
    }

    pub(crate) fn take_mesh_rebuilds(&mut self) -> Vec<ChunkDerivedWork> {
        self.derived_work
            .take_up_to(ChunkDerivedWorkKind::MeshRebuild, usize::MAX)
    }

    pub(crate) fn take_render_light_uploads(&mut self) -> Vec<ChunkDerivedWork> {
        self.derived_work
            .take_up_to(ChunkDerivedWorkKind::RenderLightUpload, usize::MAX)
    }

    pub(crate) fn take_collider_rebuilds(&mut self) -> Vec<ChunkDerivedWork> {
        self.derived_work
            .take_up_to(ChunkDerivedWorkKind::ColliderRebuild, usize::MAX)
    }

    pub(crate) fn requeue_mesh_rebuild(&mut self, work: ChunkDerivedWork) -> bool {
        self.requeue_published_derived_work(work, ChunkDerivedWorkKind::MeshRebuild)
    }

    pub(crate) fn requeue_render_light_upload(&mut self, work: ChunkDerivedWork) -> bool {
        self.requeue_published_derived_work(work, ChunkDerivedWorkKind::RenderLightUpload)
    }

    pub(crate) fn requeue_collider_rebuild(&mut self, work: ChunkDerivedWork) -> bool {
        self.requeue_published_derived_work(work, ChunkDerivedWorkKind::ColliderRebuild)
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by diagnostics and lifecycle tests")
    )]
    pub(crate) fn pending_mesh_rebuild_count(&self) -> usize {
        self.derived_work
            .effect_count(ChunkDerivedWorkKind::MeshRebuild)
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by diagnostics and lifecycle tests")
    )]
    pub(crate) fn pending_render_light_upload_count(&self) -> usize {
        self.derived_work
            .effect_count(ChunkDerivedWorkKind::RenderLightUpload)
    }

    pub(crate) fn pending_mesh_rebuilds(&self) -> impl Iterator<Item = ChunkDerivedWork> + '_ {
        self.derived_work.pending(ChunkDerivedWorkKind::MeshRebuild)
    }

    pub(crate) fn pending_render_light_uploads(
        &self,
    ) -> impl Iterator<Item = ChunkDerivedWork> + '_ {
        self.derived_work
            .pending(ChunkDerivedWorkKind::RenderLightUpload)
    }

    pub(crate) fn pending_collider_rebuilds(&self) -> impl Iterator<Item = ChunkDerivedWork> + '_ {
        self.derived_work
            .pending(ChunkDerivedWorkKind::ColliderRebuild)
    }

    pub(crate) fn has_pending_mesh_rebuild(&self, position: ChunkPos) -> bool {
        self.has_pending_derived_work(position, ChunkDerivedWorkKind::MeshRebuild)
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by diagnostics and lifecycle tests")
    )]
    pub(crate) fn has_pending_render_light_upload(&self, position: ChunkPos) -> bool {
        self.has_pending_derived_work(position, ChunkDerivedWorkKind::RenderLightUpload)
    }

    pub(crate) fn has_pending_collider_rebuild(&self, position: ChunkPos) -> bool {
        self.has_pending_derived_work(position, ChunkDerivedWorkKind::ColliderRebuild)
    }

    /// Drops every disposable task owned by this dimension.
    ///
    /// Durable save obligations live outside this queue and are unaffected.
    pub(crate) fn clear_disposable_work(&mut self) {
        self.derived_work.clear();
    }

    fn enqueue_published_derived_work(
        &mut self,
        position: ChunkPos,
        kind: ChunkDerivedWorkKind,
    ) -> bool {
        let Some(expected_entity) = self.published_chunk_entity(position) else {
            return false;
        };
        self.derived_work
            .record(position, expected_entity, kind.into())
    }

    fn requeue_published_derived_work(
        &mut self,
        work: ChunkDerivedWork,
        kind: ChunkDerivedWorkKind,
    ) -> bool {
        assert_eq!(
            work.effects(),
            ChunkDerivedEffects::only(kind),
            "consumer must requeue only the effect it took"
        );
        if self.published_chunk_entity(work.position()) != Some(work.expected_entity()) {
            return false;
        }
        self.derived_work.record(
            work.position(),
            work.expected_entity(),
            ChunkDerivedEffects::only(kind),
        )
    }

    fn has_pending_derived_work(&self, position: ChunkPos, kind: ChunkDerivedWorkKind) -> bool {
        let Some(work) = self.derived_work.get(position) else {
            return false;
        };
        self.published_chunk_entity(position) == Some(work.expected_entity())
            && work.effects().contains(kind)
    }

    fn discard_unpublished_work(&mut self, position: ChunkPos, expected_entity: Entity) {
        let effects = ChunkDerivedEffects::only(ChunkDerivedWorkKind::MeshRebuild)
            .with(ChunkDerivedWorkKind::ColliderRebuild)
            .with(ChunkDerivedWorkKind::RenderLightUpload);
        self.derived_work.take(position, expected_entity, effects);
    }

    fn discard_chunk_derived_work(&mut self, position: ChunkPos, expected_entity: Entity) {
        self.derived_work.remove(position, expected_entity);
    }

    pub(crate) fn assert_stream_owner(&self, owner: Entity) {
        assert_eq!(
            self.stream.owner(),
            owner,
            "dimension streaming state must remain bound to its entity"
        );
    }

    pub(crate) fn has_any_loaded_chunk_in_column(&self, column: ChunkColumn) -> bool {
        (0..self.height().chunks_i32()).any(|y| self.contains_loaded_chunk(column.chunk(y)))
    }

    pub(crate) fn has_complete_loaded_column(&self, column: ChunkColumn) -> bool {
        (0..self.height().chunks_i32()).all(|y| self.contains_loaded_chunk(column.chunk(y)))
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
        (0..self.height().chunks_i32())
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
        assert_eq!(entities.len(), self.height().chunks());
        assert_eq!(ticket.owner(), self.stream.owner());
        assert!(
            (0..self.height().chunks_i32())
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
            || (0..self.height().chunks_i32())
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
            || !(0..self.height().chunks_i32())
                .all(|y| self.published_chunks.contains(&column.chunk(y)))
        {
            return false;
        }
        for y in 0..self.height().chunks_i32() {
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
            || (0..self.height().chunks_i32())
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
            || !(0..self.height().chunks_i32())
                .all(|y| self.published_chunks.contains(&column.chunk(y)))
            || !self.stream.unpublish(column)
        {
            return false;
        }
        let chunks = self
            .complete_loaded_column(column)
            .expect("unpublished column must remain complete");
        for (position, entity) in chunks {
            self.discard_unpublished_work(position, entity);
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
            self.discard_chunk_derived_work(*position, *entity);
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

    /// Removes all streamed runtime data while retaining this logical
    /// dimension and its monotonic stream authority.
    ///
    /// Dirty chunks must be captured by persistence before calling this. The
    /// returned incarnation roots still own the ECS entities and must be
    /// despawned by the caller.
    pub(crate) fn drain_streamed_columns(&mut self) -> Vec<EvictedColumn> {
        assert!(
            self.loaded_chunks
                .keys()
                .all(|position| self.loaded_columns.contains_key(&position.column())),
            "dimension drain only supports chunks owned by streamed column incarnations"
        );
        assert!(
            self.published_chunks
                .iter()
                .all(|position| self.loaded_columns.contains_key(&position.column())),
            "dimension drain only supports published streamed columns"
        );
        assert!(
            self.loaded_columns.keys().all(|&column| {
                self.complete_loaded_column(column)
                    .is_some_and(|chunks| chunks.len() == self.height().chunks())
                    && self.stream.retains_loaded_column(column)
            }),
            "dimension drain requires every streamed column to remain complete and resident"
        );
        assert!(
            self.stream.columns().all(|column| {
                !self.stream.retains_loaded_column(column)
                    || self.loaded_columns.contains_key(&column)
            }),
            "dimension drain requires resident stream state to retain its column incarnation"
        );
        assert!(
            self.loaded_columns.keys().all(|&column| {
                let published = (0..self.height().chunks_i32())
                    .filter(|&y| self.published_chunks.contains(&column.chunk(y)))
                    .count();
                match self
                    .stream
                    .retained_column_state(column)
                    .map(ResidentColumnState::exposure)
                {
                    Some(ColumnExposure::Published) => published == self.height().chunks(),
                    Some(ColumnExposure::Staged) => published == 0,
                    None => false,
                }
            }),
            "dimension drain requires registry publication to match stream exposure"
        );

        if let Some(ticket) = self.light_tasks.active_ticket() {
            self.cancel_column_light_patch(ticket);
        }

        let published = self
            .stream
            .columns()
            .filter(|&column| self.column_exposure(column) == Some(ColumnExposure::Published))
            .collect::<Vec<_>>();
        for column in published {
            assert!(
                self.unpublish_column(column),
                "published streamed column must unpublish during drain"
            );
        }

        let eviction_tickets = self.stream.mark_all_undesired();
        let mut evicted = Vec::with_capacity(eviction_tickets.len());
        for ticket in eviction_tickets {
            evicted.push(
                self.evict_column(ticket)
                    .expect("current streamed eviction must commit during drain"),
            );
        }
        self.clear_disposable_work();

        assert!(
            self.stream.is_empty(),
            "dimension stream ledger must be empty after drain"
        );
        assert!(
            self.loaded_columns.is_empty()
                && self.loaded_chunks.is_empty()
                && self.published_chunks.is_empty(),
            "dimension registries must be empty after drain"
        );
        evicted
    }
}

pub struct DimensionPlugin;

impl Plugin for DimensionPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ChunkTaskPool::global())
            .init_resource::<ColumnLoadBudget>()
            .init_resource::<ColumnStagingBudget>()
            .init_resource::<ColumnActivationBudget>()
            .init_resource::<ColumnLightBudget>()
            .init_resource::<ChunkSaveBudget>()
            .init_resource::<ChunkSaveTasks>()
            .init_resource::<ViewDistance>()
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
            (finish_chunk_save_tasks, start_chunk_save_tasks).chain(),
        );
        switching::install(app);
    }
}

fn setup(mut commands: Commands, catalog: Res<DimensionCatalog>) {
    for &definition in catalog.definitions() {
        let entity = commands.spawn_empty().id();
        let active = definition.id() == DimensionId::OVERWORLD;
        let mut root = commands.entity(entity);
        root.insert(Dimension::root_components(entity, definition));
        if active {
            root.insert(Active);
        } else {
            root.insert(Visibility::Hidden);
        }
    }
}

#[derive(Component)]
pub struct Active;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immediate_spawn_binds_streaming_state_to_the_allocated_entity() {
        let mut world = World::new();

        let catalog = DimensionCatalog::for_world(&crate::world::WorldMetadata::default());
        let definition = *catalog.get(DimensionId::OVERWORLD).unwrap();
        let owner = Dimension::spawn_in_world(&mut world, &catalog, DimensionId::OVERWORLD);

        let dimension = world.get::<Dimension>(owner).unwrap();
        dimension.assert_stream_owner(owner);
        assert_eq!(dimension.definition(), definition);
        assert_eq!(dimension.id(), DimensionId::OVERWORLD);
        assert!(world.get::<DesiredColumnView>(owner).is_some());
        assert!(world.get::<Transform>(owner).is_some());
        assert!(world.get::<Visibility>(owner).is_some());
    }

    #[test]
    fn setup_spawns_every_catalog_root_but_activates_only_the_overworld() {
        let catalog = DimensionCatalog::for_world(&crate::world::WorldMetadata::default());
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(catalog.clone())
            .add_systems(Update, setup);

        app.update();

        let mut roots = app
            .world_mut()
            .query::<(Entity, &Dimension, &Visibility, Has<Active>)>();
        let roots = roots
            .iter(app.world())
            .map(|(entity, dimension, visibility, active)| {
                (entity, dimension.id(), *visibility, active)
            })
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), catalog.definitions().len());
        for definition in catalog.definitions() {
            let (_, _, visibility, active) = roots
                .iter()
                .find(|(_, id, _, _)| *id == definition.id())
                .copied()
                .expect("every catalog dimension must have a root");
            assert_eq!(active, definition.id() == DimensionId::OVERWORLD);
            assert_eq!(
                visibility,
                if active {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                }
            );
        }
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
        assert!(dimension.enqueue_mesh_rebuild(position));
        assert!(dimension.enqueue_render_light_upload(position));
        assert_eq!(dimension.pending_mesh_rebuild_count(), 1);
        assert_eq!(dimension.pending_render_light_upload_count(), 1);
        dimension.clear_disposable_work();
        assert_eq!(dimension.pending_mesh_rebuild_count(), 0);
        assert_eq!(dimension.pending_render_light_upload_count(), 0);
        assert!(dimension.enqueue_mesh_rebuild(position));
        assert!(dimension.enqueue_render_light_upload(position));
        assert_eq!(dimension.unregister_published_chunk(position), Some(entity));
        assert!(!dimension.contains_loaded_chunk(position));
        assert!(!dimension.contains_published_chunk(position));
        assert_eq!(dimension.pending_mesh_rebuild_count(), 0);
        assert_eq!(dimension.pending_render_light_upload_count(), 0);
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
    #[should_panic(
        expected = "dimension drain only supports chunks owned by streamed column incarnations"
    )]
    fn dimension_drain_rejects_independently_registered_chunks() {
        let mut dimension = Dimension::default();
        dimension.register_published_chunk(ChunkPos::ZERO, Entity::PLACEHOLDER);

        dimension.drain_streamed_columns();
    }

    #[test]
    fn streamed_column_publication_is_a_view_over_loaded_entities() {
        let height = WorldHeight::new(2).unwrap();
        let column = ChunkColumn::new(-4, 7);
        let entities = [Entity::from_bits(11), Entity::from_bits(12)];
        let mut dimension = Dimension::new_for_test(Entity::PLACEHOLDER, height);
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
