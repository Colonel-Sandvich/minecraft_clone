use std::mem::size_of;

use bevy::{
    platform::collections::HashMap,
    prelude::*,
    tasks::{Task, futures::check_ready},
};

use super::{Active, ChunkTaskPool, DesiredColumnView, Dimension};

use crate::world::{
    chunk::{
        Chunk, ChunkColumn, ChunkHeightmap, ChunkNeedsSave, ChunkPos, ChunkPosition, ChunkRevision,
    },
    definition::{ChunkAddress, DimensionId},
    storage::{ChunkRepository, ChunkStoreError, ChunkStoreResult},
};

const INITIAL_SAVE_RETRY_DELAY_UPDATES: u32 = 60;
const MAX_SAVE_RETRY_DELAY_UPDATES: u32 = 600;

#[derive(Resource, Default)]
pub(crate) struct ChunkSaveTasks {
    tasks: HashMap<ChunkSaveHandle, InFlightChunkSave>,
    failures: HashMap<ChunkSaveHandle, ChunkSaveFailure>,
    cursor: Option<ChunkSaveHandle>,
}

impl ChunkSaveTasks {
    pub(crate) fn stats(&self) -> ChunkSaveTaskStats {
        let tasks = self.tasks.len();
        let failures = self.failures.len();
        ChunkSaveTaskStats {
            tasks,
            failures,
            estimated_payload_bytes: tasks
                .saturating_mul(
                    size_of::<ChunkSaveHandle>()
                        + size_of::<InFlightChunkSave>()
                        + size_of::<ChunkSaveRequest>(),
                )
                .saturating_add(
                    failures.saturating_mul(
                        size_of::<ChunkSaveHandle>() + size_of::<ChunkSaveFailure>(),
                    ),
                ),
        }
    }

    fn tick_retry_backoffs(&mut self) {
        for failure in self.failures.values_mut() {
            let Some(delay) = &mut failure.retry_after_updates else {
                continue;
            };
            *delay = delay.saturating_sub(1);
        }
    }

    fn can_start(&self, handle: ChunkSaveHandle) -> bool {
        !self.tasks.contains_key(&handle)
            // Storage owns one shared heightmap per XZ column.
            && !self
                .tasks
                .keys()
                .any(|in_flight| in_flight.column() == handle.column())
            && self
                .failures
                .get(&handle)
                .is_none_or(|failure| failure.can_retry())
    }

    fn record_success(&mut self, handle: ChunkSaveHandle) {
        self.failures.remove(&handle);
    }

    fn record_failure(&mut self, handle: ChunkSaveHandle, error: ChunkStoreError) {
        let attempts = self
            .failures
            .get(&handle)
            .map_or(0, |failure| failure.attempts)
            .saturating_add(1);
        let retry_after_updates = error
            .is_transient()
            .then(|| retry_delay_for_attempt(attempts));

        self.failures.insert(
            handle,
            ChunkSaveFailure {
                error,
                attempts,
                retry_after_updates,
            },
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ChunkSaveHandle {
    owner: Entity,
    entity: Entity,
    position: ChunkPos,
}

impl ChunkSaveHandle {
    const fn column(self) -> ChunkColumn {
        ChunkColumn::from_chunk(self.position)
    }

    const fn order_key(self) -> (i32, i32, i32, Entity) {
        (
            self.position.x(),
            self.position.z(),
            self.position.y(),
            self.entity,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkSaveTicket {
    handle: ChunkSaveHandle,
    revision: ChunkRevision,
    heightmap: ChunkHeightmap,
}

struct InFlightChunkSave {
    ticket: ChunkSaveTicket,
    task: Task<ChunkStoreResult<()>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChunkSaveTaskStats {
    pub(crate) tasks: usize,
    pub(crate) failures: usize,
    pub(crate) estimated_payload_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChunkSaveFailure {
    error: ChunkStoreError,
    attempts: u32,
    retry_after_updates: Option<u32>,
}

impl ChunkSaveFailure {
    fn can_retry(&self) -> bool {
        self.retry_after_updates == Some(0)
    }
}

fn retry_delay_for_attempt(attempts: u32) -> u32 {
    INITIAL_SAVE_RETRY_DELAY_UPDATES
        .saturating_mul(2_u32.saturating_pow(attempts.saturating_sub(1).min(5)))
        .min(MAX_SAVE_RETRY_DELAY_UPDATES)
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
    position: ChunkPos,
    chunk: Chunk,
    heightmap: ChunkHeightmap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkSaveCandidate {
    handle: ChunkSaveHandle,
    revision: ChunkRevision,
    eviction_priority: bool,
}

pub(crate) fn finish_chunk_save_tasks(
    mut commands: Commands,
    mut save_tasks: ResMut<ChunkSaveTasks>,
    dimensions: Query<&Dimension>,
    chunks: Query<(
        &ChunkPosition,
        &Chunk,
        &ChunkHeightmap,
        Option<&ChunkNeedsSave>,
    )>,
) {
    let mut completed = Vec::new();
    for (&handle, in_flight) in save_tasks.tasks.iter_mut() {
        if let Some(result) = check_ready(&mut in_flight.task) {
            completed.push((handle, in_flight.ticket, result));
        }
    }

    for (handle, ticket, result) in completed {
        let removed = save_tasks
            .tasks
            .remove(&handle)
            .expect("completed chunk save task must remain registered");
        assert_eq!(
            removed.ticket, ticket,
            "completed chunk save ticket must match its registered task"
        );

        match result {
            Ok(()) => {
                save_tasks.record_success(handle);
                let Ok(dimension) = dimensions.get(handle.owner) else {
                    continue;
                };
                if dimension.loaded_chunk_entity(handle.position) != Some(handle.entity) {
                    continue;
                }
                let Ok((position, chunk, heightmap, Some(_))) = chunks.get(handle.entity) else {
                    continue;
                };
                if position.chunk_pos() != handle.position {
                    continue;
                }

                if chunk.content_revision() == ticket.revision && *heightmap == ticket.heightmap {
                    commands.entity(handle.entity).remove::<ChunkNeedsSave>();
                }
            }
            Err(error) => {
                warn!(%error, pos = ?handle.position, owner = ?handle.owner, "Failed to persist dirty chunk");
                if save_handle_is_current(handle, &dimensions, &chunks) {
                    save_tasks.record_failure(handle, error);
                }
            }
        }
    }

    save_tasks
        .failures
        .retain(|&handle, _| save_handle_is_current(handle, &dimensions, &chunks));
}

pub(crate) fn start_chunk_save_tasks(
    active_dimension: Option<Single<(&Dimension, Entity), With<Active>>>,
    chunks: Query<(
        &ChunkPosition,
        &Chunk,
        &ChunkHeightmap,
        Option<&ChunkNeedsSave>,
    )>,
    desired_view: Res<DesiredColumnView>,
    repository: Res<ChunkRepository>,
    save_budget: Res<ChunkSaveBudget>,
    mut save_tasks: ResMut<ChunkSaveTasks>,
    task_pool: Res<ChunkTaskPool>,
) {
    save_tasks.tick_retry_backoffs();

    let available_slots = save_budget.0.saturating_sub(save_tasks.tasks.len());
    if available_slots == 0 {
        return;
    }
    let Some(active_dimension) = active_dimension else {
        return;
    };
    let (dimension, owner) = active_dimension.into_inner();
    dimension.assert_stream_owner(owner);

    let mut candidates = Vec::new();
    for (registered_position, entity) in dimension.iter_loaded_chunks() {
        let Ok((position, chunk, _, Some(_))) = chunks.get(entity) else {
            continue;
        };
        assert_eq!(
            position.chunk_pos(),
            registered_position,
            "dimension registry and ChunkPosition must agree before saving"
        );
        candidates.push(ChunkSaveCandidate {
            handle: ChunkSaveHandle {
                owner,
                entity,
                position: registered_position,
            },
            revision: chunk.content_revision(),
            eviction_priority: !desired_view.contains_resident_column(registered_position.into()),
        });
    }

    let candidates = ordered_candidates(candidates, save_tasks.cursor);
    let mut started = 0;
    for candidate in candidates {
        if started == available_slots {
            break;
        }
        if !save_tasks.can_start(candidate.handle) {
            continue;
        }

        let (position, chunk, heightmap, Some(_)) = chunks
            .get(candidate.handle.entity)
            .expect("selected dirty chunk must remain available while starting its save")
        else {
            continue;
        };
        assert_eq!(position.chunk_pos(), candidate.handle.position);
        if chunk.content_revision() != candidate.revision {
            continue;
        }

        let ticket = ChunkSaveTicket {
            handle: candidate.handle,
            revision: candidate.revision,
            heightmap: *heightmap,
        };
        let request = ChunkSaveRequest {
            position: candidate.handle.position,
            chunk: chunk.clone(),
            heightmap: *heightmap,
        };
        let repository = repository.clone();
        let task = task_pool.spawn(async move { save_chunk_snapshot(request, repository) });
        save_tasks
            .tasks
            .insert(candidate.handle, InFlightChunkSave { ticket, task });
        save_tasks.cursor = Some(candidate.handle);
        started += 1;
    }
}

fn save_handle_is_current(
    handle: ChunkSaveHandle,
    dimensions: &Query<&Dimension>,
    chunks: &Query<(
        &ChunkPosition,
        &Chunk,
        &ChunkHeightmap,
        Option<&ChunkNeedsSave>,
    )>,
) -> bool {
    let Ok(dimension) = dimensions.get(handle.owner) else {
        return false;
    };
    if dimension.loaded_chunk_entity(handle.position) != Some(handle.entity) {
        return false;
    }
    matches!(
        chunks.get(handle.entity),
        Ok((position, _, _, Some(_))) if position.chunk_pos() == handle.position
    )
}

fn ordered_candidates(
    candidates: Vec<ChunkSaveCandidate>,
    cursor: Option<ChunkSaveHandle>,
) -> Vec<ChunkSaveCandidate> {
    let (mut eviction, mut resident): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .partition(|candidate| candidate.eviction_priority);
    rotate_after_cursor(&mut eviction, cursor);
    rotate_after_cursor(&mut resident, cursor);
    eviction.extend(resident);
    eviction
}

fn rotate_after_cursor(candidates: &mut [ChunkSaveCandidate], cursor: Option<ChunkSaveHandle>) {
    candidates.sort_unstable_by_key(|candidate| candidate.handle.order_key());
    let Some(cursor) = cursor else {
        return;
    };
    let start =
        candidates.partition_point(|candidate| candidate.handle.order_key() <= cursor.order_key());
    if !candidates.is_empty() {
        candidates.rotate_left(start % candidates.len());
    }
}

fn save_chunk_snapshot(
    request: ChunkSaveRequest,
    repository: ChunkRepository,
) -> ChunkStoreResult<()> {
    // Runtime dimension ownership is introduced in the next migration; the
    // sole active root is currently overworld.
    let address = ChunkAddress::new(DimensionId::OVERWORLD, request.position);
    repository.save_chunk(address, &request.chunk, &request.heightmap)
}

#[cfg(test)]
mod tests;
