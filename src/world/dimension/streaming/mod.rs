mod state;
mod systems;

#[cfg(test)]
mod tests;

use std::mem::size_of;

use bevy::{
    platform::collections::HashMap,
    prelude::*,
    tasks::{Task, futures::check_ready},
};

use crate::world::{chunk::ChunkColumn, loading::ColumnLoadResult};

pub(crate) use state::{
    ColumnEvictionTicket, ColumnExposure, ColumnLightRevision, ColumnLighting, ColumnLoadTicket,
    ColumnResidency, ColumnResidencyLedger, LightPatchTicket, ResidentColumnState,
};
pub(crate) use systems::{
    finish_column_loads, maintain_column_residency, publish_lit_columns,
    refresh_desired_column_view, start_column_loads,
};

#[derive(Resource, Debug, Clone, Copy)]
pub struct ColumnLoadBudget(pub usize);

impl Default for ColumnLoadBudget {
    fn default() -> Self {
        Self(4)
    }
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct ColumnStagingBudget(pub usize);

impl Default for ColumnStagingBudget {
    fn default() -> Self {
        Self(8)
    }
}

#[derive(Resource, Debug, Clone, Copy)]
pub struct ColumnActivationBudget(pub usize);

impl Default for ColumnActivationBudget {
    fn default() -> Self {
        Self(8)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ColumnLoadTaskStats {
    pub(crate) tasks: usize,
    pub(crate) failures: usize,
    pub(crate) estimated_payload_bytes: usize,
}

/// Asynchronous streaming work owned by exactly one dimension.
pub(crate) struct DimensionStreamState {
    ledger: ColumnResidencyLedger,
    load_tasks: HashMap<ColumnLoadTicket, Task<ColumnLoadResult>>,
}

impl DimensionStreamState {
    pub(crate) fn new(owner: Entity) -> Self {
        Self {
            ledger: ColumnResidencyLedger::new(owner),
            load_tasks: HashMap::new(),
        }
    }

    pub(crate) const fn owner(&self) -> Entity {
        self.ledger.owner()
    }

    pub(crate) fn mark_desired(&mut self, column: ChunkColumn) {
        self.ledger.mark_desired(column);
    }

    pub(crate) fn mark_undesired(&mut self, column: ChunkColumn) -> Option<ColumnEvictionTicket> {
        if let Some(ticket) = self.ledger.loading_ticket(column) {
            self.load_tasks.remove(&ticket);
        }
        self.ledger.mark_undesired(column)
    }

    pub(crate) fn start_load(
        &mut self,
        column: ChunkColumn,
        view_revision: u64,
        start: impl FnOnce() -> Task<ColumnLoadResult>,
    ) -> Option<ColumnLoadTicket> {
        let ticket = self.ledger.begin_load(column, view_revision)?;
        let previous = self.load_tasks.insert(ticket, start());
        debug_assert!(previous.is_none());
        Some(ticket)
    }

    pub(crate) fn take_ready_load(
        &mut self,
        column: ChunkColumn,
    ) -> Option<(ColumnLoadTicket, ColumnLoadResult)> {
        let ticket = self.ledger.loading_ticket(column)?;
        let output = check_ready(self.load_tasks.get_mut(&ticket)?)?;
        self.load_tasks.remove(&ticket);
        Some((ticket, output))
    }

    pub(crate) fn accept_load(&mut self, ticket: ColumnLoadTicket) -> bool {
        self.ledger.accept_load(ticket)
    }

    pub(crate) fn activate_load(&mut self, ticket: ColumnLoadTicket) -> bool {
        self.ledger.activate_load(ticket)
    }

    pub(crate) fn mark_light_pending(&mut self, column: ChunkColumn) -> bool {
        self.ledger.mark_light_pending(column)
    }

    pub(crate) fn begin_light_patch(
        &mut self,
        commit_columns: &[ChunkColumn],
    ) -> Option<LightPatchTicket> {
        self.ledger.begin_light_patch(commit_columns)
    }

    pub(crate) fn finish_light_patch(
        &mut self,
        ticket: LightPatchTicket,
    ) -> Option<Vec<(ChunkColumn, ColumnLightRevision)>> {
        self.ledger.finish_light_patch(ticket)
    }

    pub(crate) fn cancel_light_patch(&mut self, ticket: LightPatchTicket) -> bool {
        self.ledger.cancel_light_patch(ticket)
    }

    pub(crate) fn light_patch_ticket(&self, column: ChunkColumn) -> Option<LightPatchTicket> {
        self.ledger.light_patch_ticket(column)
    }

    pub(crate) fn light_patch_columns(&self, ticket: LightPatchTicket) -> Option<&[ChunkColumn]> {
        self.ledger.light_patch_columns(ticket)
    }

    pub(crate) fn publish(&mut self, column: ChunkColumn) -> bool {
        self.ledger.publish(column)
    }

    pub(crate) fn unpublish(&mut self, column: ChunkColumn) -> bool {
        self.ledger.unpublish(column)
    }

    pub(crate) fn resident_state(&self, column: ChunkColumn) -> Option<ResidentColumnState> {
        self.ledger.resident_state(column)
    }

    pub(crate) fn column_lighting(&self, column: ChunkColumn) -> Option<ColumnLighting> {
        self.resident_state(column)
            .map(ResidentColumnState::lighting)
    }

    pub(crate) fn column_exposure(&self, column: ChunkColumn) -> Option<ColumnExposure> {
        self.resident_state(column)
            .map(ResidentColumnState::exposure)
    }

    pub(crate) fn fail_load(
        &mut self,
        ticket: ColumnLoadTicket,
        error: crate::world::loading::ChunkLoadError,
    ) -> bool {
        self.ledger.fail_load(ticket, error)
    }

    pub(crate) fn commit_eviction(&mut self, ticket: ColumnEvictionTicket) -> bool {
        self.ledger.commit_eviction(ticket)
    }

    pub(crate) fn tick_backoffs(&mut self) {
        self.ledger.tick_backoffs();
    }

    #[cfg(test)]
    pub(crate) fn state(&self, column: ChunkColumn) -> Option<&ColumnResidency> {
        self.ledger.state(column)
    }

    pub(crate) fn columns(&self) -> impl ExactSizeIterator<Item = ChunkColumn> + '_ {
        self.ledger.states().map(|(column, _)| column)
    }

    pub(crate) fn eviction_tickets(&self) -> impl Iterator<Item = ColumnEvictionTicket> + '_ {
        self.ledger.states().filter_map(|(_, state)| match state {
            ColumnResidency::Evicting { ticket, .. } => Some(*ticket),
            _ => None,
        })
    }

    pub(crate) fn loading_count(&self) -> usize {
        self.load_tasks.len()
    }

    pub(crate) fn stats(&self) -> ColumnLoadTaskStats {
        let residency = self.ledger.stats();
        ColumnLoadTaskStats {
            tasks: self.load_tasks.len(),
            failures: residency.failed,
            estimated_payload_bytes: self.load_tasks.len().saturating_mul(
                size_of::<ColumnLoadTicket>()
                    + size_of::<Task<ColumnLoadResult>>()
                    + size_of::<ColumnLoadResult>(),
            ),
        }
    }
}

#[cfg(test)]
mod light_patch_authority_tests {
    use super::*;

    fn activate(stream: &mut DimensionStreamState, column: ChunkColumn) {
        let ticket = stream.ledger.begin_load(column, 1).unwrap();
        assert!(stream.accept_load(ticket));
        assert!(stream.activate_load(ticket));
    }

    #[test]
    fn stream_state_exposes_shared_light_patch_authority() {
        let first = ChunkColumn::new(1, 2);
        let second = ChunkColumn::new(2, 2);
        let mut stream = DimensionStreamState::new(Entity::PLACEHOLDER);
        activate(&mut stream, first);
        activate(&mut stream, second);

        let ticket = stream.begin_light_patch(&[first, second]).unwrap();
        assert_eq!(stream.light_patch_ticket(first), Some(ticket));
        assert_eq!(stream.light_patch_ticket(second), Some(ticket));
        assert_eq!(
            stream.light_patch_columns(ticket),
            Some(&[first, second][..])
        );
        let revisions = stream.finish_light_patch(ticket).unwrap();
        assert_eq!(
            revisions
                .iter()
                .map(|(column, _)| *column)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
        for (column, revision) in revisions {
            assert!(revision > ColumnLightRevision::INITIAL);
            assert_eq!(
                stream
                    .resident_state(column)
                    .map(|state| state.light_revision()),
                Some(revision)
            );
        }

        assert!(stream.mark_light_pending(first));
        assert!(stream.mark_light_pending(second));
        let cancelled = stream.begin_light_patch(&[first, second]).unwrap();
        assert!(stream.cancel_light_patch(cancelled));
        assert_eq!(stream.column_lighting(first), Some(ColumnLighting::Pending));
        assert_eq!(
            stream.column_lighting(second),
            Some(ColumnLighting::Pending)
        );
    }
}
