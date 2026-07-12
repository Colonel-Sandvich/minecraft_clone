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
    ColumnEvictionTicket, ColumnLoadTicket, ColumnResidency, ColumnResidencyLedger,
};
pub(crate) use systems::{
    finish_column_loads, maintain_column_residency, refresh_desired_column_view, start_column_loads,
};

#[derive(Resource, Debug, Clone, Copy)]
pub struct ColumnLoadBudget(pub usize);

impl Default for ColumnLoadBudget {
    fn default() -> Self {
        Self(4)
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
            ColumnResidency::Evicting { ticket } => Some(*ticket),
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
