use std::mem::size_of;

use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
    tasks::Task,
};

use crate::world::loading::{ChunkLoadError, ChunkLoadOutput};

const INITIAL_LOAD_RETRY_DELAY_UPDATES: u32 = 30;
const MAX_LOAD_RETRY_DELAY_UPDATES: u32 = 600;

#[derive(Resource, Default)]
pub(crate) struct ChunkLoadTasks {
    pub(crate) tasks: HashMap<IVec3, Task<ChunkLoadOutput>>,
    pub(crate) failures: HashMap<IVec3, ChunkLoadFailure>,
}

impl ChunkLoadTasks {
    pub(crate) fn stats(&self) -> ChunkLoadTaskStats {
        let tasks = self.tasks.len();
        let failures = self.failures.len();
        ChunkLoadTaskStats {
            tasks,
            failures,
            estimated_payload_bytes: tasks
                .saturating_mul(
                    size_of::<IVec3>()
                        + size_of::<Task<ChunkLoadOutput>>()
                        + size_of::<ChunkLoadOutput>(),
                )
                .saturating_add(
                    failures.saturating_mul(size_of::<IVec3>() + size_of::<ChunkLoadFailure>()),
                ),
        }
    }

    pub(crate) fn retain_visible(&mut self, chunks_in_view: &HashSet<IVec3>) {
        self.tasks.retain(|pos, _| chunks_in_view.contains(pos));
        self.failures.retain(|pos, _| chunks_in_view.contains(pos));
    }

    pub(crate) fn tick_failure_backoffs(&mut self) {
        for failure in self.failures.values_mut() {
            let Some(delay) = &mut failure.retry_after_updates else {
                continue;
            };

            *delay = delay.saturating_sub(1);
        }
    }

    pub(crate) fn blocks_starting_task(&self, pos: IVec3) -> bool {
        if self.tasks.contains_key(&pos) {
            return true;
        }

        self.failures
            .get(&pos)
            .is_some_and(|failure| !failure.can_retry())
    }

    pub(crate) fn record_success(&mut self, pos: IVec3) {
        self.failures.remove(&pos);
    }

    pub(crate) fn record_failure(&mut self, pos: IVec3, error: ChunkLoadError) {
        let attempts = self
            .failures
            .get(&pos)
            .map_or(0, |failure| failure.attempts)
            .saturating_add(1);

        let retry_after_updates = if error.is_transient() {
            Some(retry_delay_for_attempt(attempts))
        } else {
            None
        };

        self.failures.insert(
            pos,
            ChunkLoadFailure {
                error,
                attempts,
                retry_after_updates,
            },
        );
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ChunkLoadTaskStats {
    pub(crate) tasks: usize,
    pub(crate) failures: usize,
    pub(crate) estimated_payload_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChunkLoadFailure {
    pub(crate) error: ChunkLoadError,
    pub(crate) attempts: u32,
    pub(crate) retry_after_updates: Option<u32>,
}

impl ChunkLoadFailure {
    fn can_retry(&self) -> bool {
        self.retry_after_updates == Some(0)
    }
}

fn retry_delay_for_attempt(attempts: u32) -> u32 {
    INITIAL_LOAD_RETRY_DELAY_UPDATES
        .saturating_mul(2_u32.saturating_pow(attempts.saturating_sub(1).min(5)))
        .min(MAX_LOAD_RETRY_DELAY_UPDATES)
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct ChunkLoadBudget(pub usize);

impl Default for ChunkLoadBudget {
    fn default() -> Self {
        Self(16)
    }
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct ChunkSpawnBudget(pub usize);

impl Default for ChunkSpawnBudget {
    fn default() -> Self {
        Self(32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_is_bounded_exponential() {
        assert_eq!(retry_delay_for_attempt(1), 30);
        assert_eq!(retry_delay_for_attempt(2), 60);
        assert_eq!(retry_delay_for_attempt(6), 600);
        assert_eq!(retry_delay_for_attempt(20), 600);
    }
}
