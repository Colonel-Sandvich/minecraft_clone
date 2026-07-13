use std::{mem::size_of, sync::Arc};

use bevy::{
    platform::collections::HashMap,
    prelude::*,
    tasks::{Task, futures::check_ready},
};

use super::{Active, ChunkTaskPool, DesiredColumnView, Dimension};

use crate::world::{
    chunk::{Chunk, ChunkHeightmap, ChunkNeedsSave, ChunkPosition, ChunkRevision},
    definition::ChunkAddress,
    storage::{ChunkRepository, ChunkStoreError, ChunkStoreResult},
};

const INITIAL_SAVE_RETRY_DELAY_UPDATES: u32 = 60;
const MAX_SAVE_RETRY_DELAY_UPDATES: u32 = 600;

#[derive(Resource, Default)]
pub(crate) struct ChunkSaveTasks {
    pending: HashMap<ChunkAddress, PendingChunkSave>,
    in_flight: HashMap<ChunkAddress, InFlightChunkSave>,
    failures: HashMap<ChunkAddress, ChunkSaveFailure>,
    cursor: Option<ChunkAddress>,
    next_sequence: u64,
}

impl ChunkSaveTasks {
    pub(crate) fn has_uncommitted_dimension(&self, dimension: crate::world::DimensionId) -> bool {
        self.pending
            .keys()
            .chain(self.in_flight.keys())
            .chain(self.failures.keys())
            .any(|address| address.dimension() == dimension)
    }

    pub(crate) fn retry_permanent_failure(&mut self, address: ChunkAddress) -> bool {
        let Some(failure) = self.failures.get_mut(&address) else {
            return false;
        };
        if failure.retry_after_updates.is_some() || !self.pending.contains_key(&address) {
            return false;
        }
        failure.retry_after_updates = Some(0);
        true
    }

    pub(crate) fn stats(&self) -> ChunkSaveTaskStats {
        let pending = self.pending.len();
        let in_flight = self.in_flight.len();
        let failures = self.failures.len();
        ChunkSaveTaskStats {
            tasks: pending.saturating_add(in_flight),
            failures,
            estimated_payload_bytes: pending
                .saturating_mul(
                    size_of::<ChunkAddress>()
                        + size_of::<PendingChunkSave>()
                        + size_of::<ChunkSavePayload>(),
                )
                .saturating_add(in_flight.saturating_mul(
                    size_of::<ChunkAddress>()
                        + size_of::<InFlightChunkSave>()
                        + size_of::<ChunkSavePayload>(),
                ))
                .saturating_add(
                    failures
                        .saturating_mul(size_of::<ChunkAddress>() + size_of::<ChunkSaveFailure>()),
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

    fn can_start(&self, address: ChunkAddress) -> bool {
        let Some(pending) = self.pending.get(&address) else {
            return false;
        };
        !self.in_flight.contains_key(&address)
            // Storage owns one shared heightmap per XZ column.
            && !self
                .in_flight
                .keys()
                .any(|in_flight| in_flight.column() == address.column())
            && !self.pending.iter().any(|(&other_address, other)| {
                other_address.column() == address.column()
                    && other.snapshot.sequence < pending.snapshot.sequence
            })
            && self
                .failures
                .get(&address)
                .is_none_or(|failure| failure.can_retry())
    }

    fn record_success(&mut self, address: ChunkAddress) {
        self.failures.remove(&address);
    }

    fn record_failure(&mut self, address: ChunkAddress, error: ChunkStoreError) {
        let attempts = self
            .failures
            .get(&address)
            .map_or(0, |failure| failure.attempts)
            .saturating_add(1);
        let retry_after_updates = error
            .is_transient()
            .then(|| retry_delay_for_attempt(attempts));

        self.failures.insert(
            address,
            ChunkSaveFailure {
                error,
                attempts,
                retry_after_updates,
            },
        );
    }

    fn capture_live_snapshot(
        &mut self,
        address: ChunkAddress,
        source: LiveChunkSaveSource,
        chunk: &Chunk,
        heightmap: ChunkHeightmap,
        eviction_priority: bool,
    ) {
        let ticket = ChunkSaveTicket {
            source: Some(source),
            heightmap,
        };

        if let Some(pending) = self.pending.get_mut(&address)
            && pending.snapshot.ticket() == ticket
        {
            pending.eviction_priority = eviction_priority;
            return;
        }

        if let Some(in_flight) = self.in_flight.get_mut(&address)
            && in_flight.snapshot.ticket() == ticket
        {
            in_flight.eviction_priority = eviction_priority;
            self.pending.remove(&address);
            return;
        }

        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("chunk save capture sequence exhausted");
        self.pending.insert(
            address,
            PendingChunkSave {
                snapshot: OwnedChunkSaveSnapshot {
                    sequence,
                    payload: Arc::new(ChunkSavePayload {
                        chunk: chunk.clone(),
                        heightmap,
                    }),
                    source: Some(source),
                },
                eviction_priority,
            },
        );
    }

    fn requeue_failed_snapshot(
        &mut self,
        address: ChunkAddress,
        snapshot: OwnedChunkSaveSnapshot,
        eviction_priority: bool,
    ) {
        // Anything captured while this task was running is newer than the
        // failed payload and must remain the retry candidate.
        self.pending.entry(address).or_insert(PendingChunkSave {
            snapshot,
            eviction_priority,
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LiveChunkSaveSource {
    owner: Entity,
    entity: Entity,
    revision: ChunkRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedChunkSaveSnapshot {
    sequence: u64,
    payload: Arc<ChunkSavePayload>,
    source: Option<LiveChunkSaveSource>,
}

impl OwnedChunkSaveSnapshot {
    fn ticket(&self) -> ChunkSaveTicket {
        ChunkSaveTicket {
            source: self.source,
            heightmap: self.payload.heightmap,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChunkSavePayload {
    chunk: Chunk,
    heightmap: ChunkHeightmap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkSaveTicket {
    source: Option<LiveChunkSaveSource>,
    heightmap: ChunkHeightmap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingChunkSave {
    snapshot: OwnedChunkSaveSnapshot,
    eviction_priority: bool,
}

struct InFlightChunkSave {
    snapshot: OwnedChunkSaveSnapshot,
    eviction_priority: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkSaveCandidate {
    address: ChunkAddress,
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
    for (&address, in_flight) in save_tasks.in_flight.iter_mut() {
        if let Some(result) = check_ready(&mut in_flight.task) {
            completed.push((address, result));
        }
    }

    for (address, result) in completed {
        let removed = save_tasks
            .in_flight
            .remove(&address)
            .expect("completed chunk save task must remain registered");

        match result {
            Ok(()) => {
                save_tasks.record_success(address);
                clear_live_source_if_current(
                    &mut commands,
                    address,
                    &removed.snapshot,
                    &dimensions,
                    &chunks,
                );
            }
            Err(error) => {
                warn!(%error, ?address, "Failed to persist owned chunk snapshot");
                save_tasks.record_failure(address, error);
                save_tasks.requeue_failed_snapshot(
                    address,
                    removed.snapshot,
                    removed.eviction_priority,
                );
            }
        }
    }
}

pub(crate) fn start_chunk_save_tasks(
    active_dimension: Option<Single<(&Dimension, &DesiredColumnView, Entity), With<Active>>>,
    chunks: Query<(
        &ChunkPosition,
        &Chunk,
        &ChunkHeightmap,
        Option<&ChunkNeedsSave>,
    )>,
    repository: Res<ChunkRepository>,
    save_budget: Res<ChunkSaveBudget>,
    mut save_tasks: ResMut<ChunkSaveTasks>,
    task_pool: Res<ChunkTaskPool>,
) {
    save_tasks.tick_retry_backoffs();

    if let Some(active_dimension) = active_dimension {
        let (dimension, desired_view, owner) = active_dimension.into_inner();
        capture_dimension_save_snapshots(&mut save_tasks, dimension, desired_view, owner, &chunks);
    }

    let available_slots = save_budget.0.saturating_sub(save_tasks.in_flight.len());
    if available_slots == 0 {
        return;
    }

    let candidates = save_tasks
        .pending
        .iter()
        .map(|(&address, pending)| ChunkSaveCandidate {
            address,
            eviction_priority: pending.eviction_priority,
        })
        .collect();
    let candidates = ordered_candidates(candidates, save_tasks.cursor);
    let mut started = 0;
    for candidate in candidates {
        if started == available_slots {
            break;
        }
        if !save_tasks.can_start(candidate.address) {
            continue;
        }

        let pending = save_tasks
            .pending
            .remove(&candidate.address)
            .expect("selected pending chunk save must remain registered");
        let repository = repository.clone();
        let address = candidate.address;
        let worker_snapshot = pending.snapshot.clone();
        let task = task_pool
            .spawn(async move { save_chunk_snapshot(address, &worker_snapshot, &repository) });
        save_tasks.in_flight.insert(
            address,
            InFlightChunkSave {
                snapshot: pending.snapshot,
                eviction_priority: pending.eviction_priority,
                task,
            },
        );
        save_tasks.cursor = Some(address);
        started += 1;
    }
}

/// Transfers every dirty registered chunk in one runtime dimension into the
/// persistence queue. Switching code must call this before despawning an
/// outgoing root; capture is independent of `Active` and the I/O budget.
pub(crate) fn capture_dimension_save_snapshots(
    save_tasks: &mut ChunkSaveTasks,
    dimension: &Dimension,
    desired_view: &DesiredColumnView,
    owner: Entity,
    chunks: &Query<(
        &ChunkPosition,
        &Chunk,
        &ChunkHeightmap,
        Option<&ChunkNeedsSave>,
    )>,
) -> usize {
    dimension.assert_stream_owner(owner);
    let mut captured = 0;
    for (registered_position, entity) in dimension.iter_loaded_chunks() {
        let Ok((position, chunk, heightmap, Some(_))) = chunks.get(entity) else {
            continue;
        };
        assert_eq!(
            position.chunk_pos(),
            registered_position,
            "dimension registry and ChunkPosition must agree before saving"
        );
        save_tasks.capture_live_snapshot(
            ChunkAddress::new(dimension.id(), registered_position),
            LiveChunkSaveSource {
                owner,
                entity,
                revision: chunk.content_revision(),
            },
            chunk,
            *heightmap,
            !desired_view.contains_resident_column(registered_position.into()),
        );
        captured += 1;
    }
    captured
}

fn clear_live_source_if_current(
    commands: &mut Commands,
    address: ChunkAddress,
    snapshot: &OwnedChunkSaveSnapshot,
    dimensions: &Query<&Dimension>,
    chunks: &Query<(
        &ChunkPosition,
        &Chunk,
        &ChunkHeightmap,
        Option<&ChunkNeedsSave>,
    )>,
) {
    let Some(source) = snapshot.source else {
        return;
    };
    let Ok(dimension) = dimensions.get(source.owner) else {
        return;
    };
    if dimension.id() != address.dimension()
        || dimension.loaded_chunk_entity(address.position()) != Some(source.entity)
    {
        return;
    }
    let Ok((position, chunk, heightmap, Some(_))) = chunks.get(source.entity) else {
        return;
    };
    if position.chunk_pos() == address.position()
        && chunk.content_revision() == source.revision
        && *heightmap == snapshot.payload.heightmap
    {
        commands.entity(source.entity).remove::<ChunkNeedsSave>();
    }
}

fn ordered_candidates(
    candidates: Vec<ChunkSaveCandidate>,
    cursor: Option<ChunkAddress>,
) -> Vec<ChunkSaveCandidate> {
    let (mut eviction, mut resident): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .partition(|candidate| candidate.eviction_priority);
    rotate_after_cursor(&mut eviction, cursor);
    rotate_after_cursor(&mut resident, cursor);
    eviction.extend(resident);
    eviction
}

fn rotate_after_cursor(candidates: &mut [ChunkSaveCandidate], cursor: Option<ChunkAddress>) {
    candidates.sort_unstable_by_key(|candidate| address_order_key(candidate.address));
    let Some(cursor) = cursor else {
        return;
    };
    let cursor = address_order_key(cursor);
    let start =
        candidates.partition_point(|candidate| address_order_key(candidate.address) <= cursor);
    if !candidates.is_empty() {
        candidates.rotate_left(start % candidates.len());
    }
}

const fn address_order_key(address: ChunkAddress) -> (u32, i32, i32, i32) {
    let position = address.position();
    (
        address.dimension().get(),
        position.x(),
        position.z(),
        position.y(),
    )
}

fn save_chunk_snapshot(
    address: ChunkAddress,
    snapshot: &OwnedChunkSaveSnapshot,
    repository: &ChunkRepository,
) -> ChunkStoreResult<()> {
    repository.save_chunk(
        address,
        &snapshot.payload.chunk,
        &snapshot.payload.heightmap,
    )
}

#[cfg(test)]
mod tests;
