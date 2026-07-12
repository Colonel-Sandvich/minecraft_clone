use bevy::{platform::collections::HashMap, prelude::Entity};

use crate::world::{chunk::ChunkColumn, loading::ChunkLoadError};

const INITIAL_LOAD_RETRY_DELAY_UPDATES: u32 = 30;
const MAX_LOAD_RETRY_DELAY_UPDATES: u32 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ColumnLoadTicket {
    owner: Entity,
    column: ChunkColumn,
    view_revision: u64,
    version: u64,
}

impl ColumnLoadTicket {
    pub(crate) const fn owner(self) -> Entity {
        self.owner
    }

    pub(crate) const fn column(self) -> ChunkColumn {
        self.column
    }

    pub(crate) const fn view_revision(self) -> u64 {
        self.view_revision
    }

    #[cfg(test)]
    pub(crate) const fn version(self) -> u64 {
        self.version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ColumnEvictionTicket {
    owner: Entity,
    column: ChunkColumn,
    version: u64,
}

impl ColumnEvictionTicket {
    pub(crate) const fn owner(self) -> Entity {
        self.owner
    }

    pub(crate) const fn column(self) -> ChunkColumn {
        self.column
    }

    #[cfg(test)]
    pub(crate) const fn version(self) -> u64 {
        self.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ColumnResidency {
    Loading {
        ticket: ColumnLoadTicket,
        attempt: u32,
        accepted: bool,
    },
    Resident(ResidentColumnState),
    Evicting {
        ticket: ColumnEvictionTicket,
        resident: ResidentColumnState,
    },
    Failed(ColumnLoadFailure),
}

/// Derived lighting readiness for a loaded column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnLighting {
    Pending,
    Lit,
}

/// Monotonic revision of the last authoritative column-light result.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ColumnLightRevision(u64);

impl ColumnLightRevision {
    pub(crate) const INITIAL: Self = Self(0);

    fn advance(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("column light revision overflowed"),
        )
    }
}

/// Whether a loaded column is exposed to gameplay and rendering consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnExposure {
    Staged,
    Published,
}

/// Orthogonal derived-data and exposure state retained for a loaded column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResidentColumnState {
    lighting: ColumnLighting,
    exposure: ColumnExposure,
    light_revision: ColumnLightRevision,
}

impl ResidentColumnState {
    pub(crate) const STAGED_PENDING: Self = Self {
        lighting: ColumnLighting::Pending,
        exposure: ColumnExposure::Staged,
        light_revision: ColumnLightRevision::INITIAL,
    };

    pub(crate) const fn lighting(self) -> ColumnLighting {
        self.lighting
    }

    pub(crate) const fn exposure(self) -> ColumnExposure {
        self.exposure
    }

    pub(crate) const fn light_revision(self) -> ColumnLightRevision {
        self.light_revision
    }

    pub(crate) const fn is_light_pending(self) -> bool {
        matches!(self.lighting, ColumnLighting::Pending)
    }

    pub(crate) const fn is_lit(self) -> bool {
        matches!(self.lighting, ColumnLighting::Lit)
    }

    pub(crate) const fn is_staged(self) -> bool {
        matches!(self.exposure, ColumnExposure::Staged)
    }

    pub(crate) const fn is_published(self) -> bool {
        matches!(self.exposure, ColumnExposure::Published)
    }

    fn mark_light_pending(&mut self) -> bool {
        if self.is_light_pending() {
            return false;
        }
        self.lighting = ColumnLighting::Pending;
        true
    }

    fn finish_lighting(&mut self) -> Option<ColumnLightRevision> {
        if !self.is_light_pending() {
            return None;
        }
        self.light_revision = self.light_revision.advance();
        self.lighting = ColumnLighting::Lit;
        Some(self.light_revision)
    }

    fn publish(&mut self) -> bool {
        if !self.is_lit() || self.is_published() {
            return false;
        }
        self.exposure = ColumnExposure::Published;
        true
    }

    fn unpublish(&mut self) -> bool {
        if self.is_staged() {
            return false;
        }
        self.exposure = ColumnExposure::Staged;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ColumnLoadFailure {
    error: ChunkLoadError,
    attempts: u32,
    retry_after_updates: Option<u32>,
}

impl ColumnLoadFailure {
    #[cfg(test)]
    pub(crate) const fn error(&self) -> &ChunkLoadError {
        &self.error
    }

    #[cfg(test)]
    pub(crate) const fn attempts(&self) -> u32 {
        self.attempts
    }

    #[cfg(test)]
    pub(crate) const fn retry_after_updates(&self) -> Option<u32> {
        self.retry_after_updates
    }

    pub(crate) fn can_retry(&self) -> bool {
        self.retry_after_updates == Some(0)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ColumnResidencyStats {
    pub(crate) loading: usize,
    pub(crate) accepted_loads: usize,
    pub(crate) resident: usize,
    pub(crate) evicting: usize,
    pub(crate) failed: usize,
    pub(crate) retryable_failures: usize,
}

/// Dimension-owned column residency and asynchronous transition authority.
///
/// This ledger deliberately stores no chunk entities. `Dimension` remains the
/// authoritative position-to-entity registry; tickets only authorize state
/// transitions performed by the ECS adapter that owns both values.
#[derive(Debug)]
pub(crate) struct ColumnResidencyLedger {
    owner: Entity,
    next_version: u64,
    states: HashMap<ChunkColumn, ColumnResidency>,
}

impl ColumnResidencyLedger {
    pub(crate) fn new(owner: Entity) -> Self {
        Self {
            owner,
            next_version: 1,
            states: HashMap::new(),
        }
    }

    pub(crate) const fn owner(&self) -> Entity {
        self.owner
    }

    /// Marks a column wanted. Re-entry cancels an outstanding eviction and
    /// restores its exact resident state without issuing a load.
    pub(crate) fn mark_desired(&mut self, column: ChunkColumn) -> bool {
        let Some(ColumnResidency::Evicting { resident, .. }) = self.states.get(&column) else {
            return false;
        };
        let resident = *resident;
        self.states
            .insert(column, ColumnResidency::Resident(resident));
        true
    }

    /// Marks a column unwanted. Loads and failures are discarded immediately;
    /// resident columns receive an owner/version-bound eviction ticket.
    pub(crate) fn mark_undesired(&mut self, column: ChunkColumn) -> Option<ColumnEvictionTicket> {
        match self.states.remove(&column) {
            Some(ColumnResidency::Resident(resident)) => {
                let ticket = self.issue_eviction_ticket(column);
                self.states
                    .insert(column, ColumnResidency::Evicting { ticket, resident });
                Some(ticket)
            }
            Some(ColumnResidency::Evicting { ticket, resident }) => {
                self.states
                    .insert(column, ColumnResidency::Evicting { ticket, resident });
                Some(ticket)
            }
            Some(ColumnResidency::Loading { .. }) | Some(ColumnResidency::Failed(_)) | None => None,
        }
    }

    /// Begins a caller-authorized load. `view_revision` records why the work
    /// was issued; ticket identity remains the authority for acceptance.
    pub(crate) fn begin_load(
        &mut self,
        column: ChunkColumn,
        view_revision: u64,
    ) -> Option<ColumnLoadTicket> {
        let attempt = match self.states.remove(&column) {
            None => 1,
            Some(ColumnResidency::Failed(failure)) if failure.can_retry() => {
                failure.attempts.saturating_add(1)
            }
            Some(state) => {
                self.states.insert(column, state);
                return None;
            }
        };

        let ticket = self.issue_load_ticket(column, view_revision);
        self.states.insert(
            column,
            ColumnResidency::Loading {
                ticket,
                attempt,
                accepted: false,
            },
        );
        Some(ticket)
    }

    /// Accepts a completed task result only while its exact owner-bound ticket
    /// is the active load attempt for the column.
    pub(crate) fn accept_load(&mut self, ticket: ColumnLoadTicket) -> bool {
        if ticket.owner != self.owner {
            return false;
        }
        let Some(ColumnResidency::Loading {
            ticket: active,
            accepted,
            ..
        }) = self.states.get_mut(&ticket.column)
        else {
            return false;
        };
        if *active != ticket {
            return false;
        }

        *accepted = true;
        true
    }

    /// Commits installation after a previously accepted result has been
    /// registered in `Dimension`. Newly installed data is staged and awaits
    /// its first lighting result before it can be published.
    pub(crate) fn activate_load(&mut self, ticket: ColumnLoadTicket) -> bool {
        if ticket.owner != self.owner {
            return false;
        }
        let can_activate = matches!(
            self.states.get(&ticket.column),
            Some(ColumnResidency::Loading {
                ticket: active,
                accepted: true,
                ..
            }) if *active == ticket
        );
        if !can_activate {
            return false;
        }

        self.states.insert(
            ticket.column,
            ColumnResidency::Resident(ResidentColumnState::STAGED_PENDING),
        );
        true
    }

    /// Invalidates derived light while preserving whether the column is
    /// currently staged or published.
    pub(crate) fn mark_light_pending(&mut self, column: ChunkColumn) -> bool {
        let Some(ColumnResidency::Resident(resident)) = self.states.get_mut(&column) else {
            return false;
        };
        resident.mark_light_pending()
    }

    /// Records an exact lighting result for a currently pending resident.
    pub(crate) fn finish_lighting(&mut self, column: ChunkColumn) -> Option<ColumnLightRevision> {
        let Some(ColumnResidency::Resident(resident)) = self.states.get_mut(&column) else {
            return None;
        };
        resident.finish_lighting()
    }

    /// Publishes a staged resident only after exact lighting is available.
    pub(crate) fn publish(&mut self, column: ChunkColumn) -> bool {
        let Some(ColumnResidency::Resident(resident)) = self.states.get_mut(&column) else {
            return false;
        };
        resident.publish()
    }

    /// Stops exposing a resident without discarding its loaded or lighting
    /// state.
    pub(crate) fn unpublish(&mut self, column: ChunkColumn) -> bool {
        let Some(ColumnResidency::Resident(resident)) = self.states.get_mut(&column) else {
            return false;
        };
        resident.unpublish()
    }

    pub(crate) fn fail_load(&mut self, ticket: ColumnLoadTicket, error: ChunkLoadError) -> bool {
        if ticket.owner != self.owner {
            return false;
        }
        let attempt = match self.states.get(&ticket.column) {
            Some(ColumnResidency::Loading {
                ticket: active,
                attempt,
                accepted: false,
            }) if *active == ticket => *attempt,
            _ => return false,
        };

        let retry_after_updates = if error.is_transient() {
            Some(retry_delay_for_attempt(attempt))
        } else {
            None
        };
        self.states.insert(
            ticket.column,
            ColumnResidency::Failed(ColumnLoadFailure {
                error,
                attempts: attempt,
                retry_after_updates,
            }),
        );
        true
    }

    /// Cancels one exact load attempt. A caller-authorized later `begin_load`
    /// receives a strictly newer ticket.
    #[cfg(test)]
    pub(crate) fn cancel_load(&mut self, ticket: ColumnLoadTicket) -> bool {
        if ticket.owner != self.owner {
            return false;
        }
        let matches = matches!(
            self.states.get(&ticket.column),
            Some(ColumnResidency::Loading { ticket: active, .. }) if *active == ticket
        );
        if matches {
            self.states.remove(&ticket.column);
        }
        matches
    }

    pub(crate) fn commit_eviction(&mut self, ticket: ColumnEvictionTicket) -> bool {
        if ticket.owner != self.owner {
            return false;
        }
        let matches = matches!(
            self.states.get(&ticket.column),
            Some(ColumnResidency::Evicting {
                ticket: active, ..
            }) if *active == ticket
        );
        if matches {
            self.states.remove(&ticket.column);
        }
        matches
    }

    pub(crate) fn tick_backoffs(&mut self) {
        for state in self.states.values_mut() {
            let ColumnResidency::Failed(failure) = state else {
                continue;
            };
            let Some(delay) = &mut failure.retry_after_updates else {
                continue;
            };
            *delay = delay.saturating_sub(1);
        }
    }

    #[cfg(test)]
    pub(crate) fn state(&self, column: ChunkColumn) -> Option<&ColumnResidency> {
        self.states.get(&column)
    }

    pub(crate) fn loading_ticket(&self, column: ChunkColumn) -> Option<ColumnLoadTicket> {
        match self.states.get(&column) {
            Some(ColumnResidency::Loading { ticket, .. }) => Some(*ticket),
            _ => None,
        }
    }

    pub(crate) fn resident_state(&self, column: ChunkColumn) -> Option<ResidentColumnState> {
        match self.states.get(&column) {
            Some(ColumnResidency::Resident(resident)) => Some(*resident),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn eviction_ticket(&self, column: ChunkColumn) -> Option<ColumnEvictionTicket> {
        match self.states.get(&column) {
            Some(ColumnResidency::Evicting { ticket, .. }) => Some(*ticket),
            _ => None,
        }
    }

    pub(crate) fn states(
        &self,
    ) -> impl ExactSizeIterator<Item = (ChunkColumn, &ColumnResidency)> + '_ {
        self.states.iter().map(|(&column, state)| (column, state))
    }

    pub(crate) fn stats(&self) -> ColumnResidencyStats {
        let mut stats = ColumnResidencyStats::default();
        for state in self.states.values() {
            match state {
                ColumnResidency::Loading { accepted, .. } => {
                    stats.loading += 1;
                    stats.accepted_loads += *accepted as usize;
                }
                ColumnResidency::Resident(_) => stats.resident += 1,
                ColumnResidency::Evicting { .. } => stats.evicting += 1,
                ColumnResidency::Failed(failure) => {
                    stats.failed += 1;
                    stats.retryable_failures += failure.can_retry() as usize;
                }
            }
        }
        stats
    }

    fn issue_load_ticket(&mut self, column: ChunkColumn, view_revision: u64) -> ColumnLoadTicket {
        ColumnLoadTicket {
            owner: self.owner,
            column,
            view_revision,
            version: self.take_version(),
        }
    }

    fn issue_eviction_ticket(&mut self, column: ChunkColumn) -> ColumnEvictionTicket {
        ColumnEvictionTicket {
            owner: self.owner,
            column,
            version: self.take_version(),
        }
    }

    fn take_version(&mut self) -> u64 {
        let version = self.next_version;
        self.next_version = self
            .next_version
            .checked_add(1)
            .expect("column residency ticket version exhausted");
        version
    }
}

fn retry_delay_for_attempt(attempts: u32) -> u32 {
    INITIAL_LOAD_RETRY_DELAY_UPDATES
        .saturating_mul(2_u32.saturating_pow(attempts.saturating_sub(1).min(5)))
        .min(MAX_LOAD_RETRY_DELAY_UPDATES)
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use crate::world::storage::ChunkStoreError;

    use super::*;

    fn transient_error() -> ChunkLoadError {
        ChunkLoadError::transient(ChunkStoreError::Io {
            kind: ErrorKind::TimedOut,
            message: "timed out".to_owned(),
        })
    }

    fn permanent_error() -> ChunkLoadError {
        ChunkLoadError::permanent(ChunkStoreError::Io {
            kind: ErrorKind::PermissionDenied,
            message: "permission denied".to_owned(),
        })
    }

    fn load_resident(
        ledger: &mut ColumnResidencyLedger,
        column: ChunkColumn,
        view_revision: u64,
    ) -> ColumnLoadTicket {
        let ticket = ledger.begin_load(column, view_revision).unwrap();
        assert!(ledger.accept_load(ticket));
        assert!(ledger.activate_load(ticket));
        ticket
    }

    #[test]
    fn load_activation_requires_the_current_accepted_owner_ticket() {
        let owner = Entity::from_bits(1 << 32 | 7);
        let other_owner = Entity::from_bits(1 << 32 | 8);
        let column = ChunkColumn::new(-4, 9);
        let mut ledger = ColumnResidencyLedger::new(owner);
        let mut other = ColumnResidencyLedger::new(other_owner);
        let ticket = ledger.begin_load(column, 17).unwrap();

        assert_eq!(ledger.owner(), owner);
        assert_eq!(ticket.owner(), owner);
        assert_eq!(ticket.column(), column);
        assert_eq!(ticket.view_revision(), 17);
        assert_eq!(ledger.loading_ticket(column), Some(ticket));
        assert_eq!(ledger.eviction_ticket(column), None);
        assert!(!ledger.activate_load(ticket));
        assert!(!other.accept_load(ticket));
        assert!(ledger.accept_load(ticket));
        assert!(!ledger.fail_load(ticket, transient_error()));
        assert!(ledger.activate_load(ticket));
        assert_eq!(
            ledger.state(column),
            Some(&ColumnResidency::Resident(
                ResidentColumnState::STAGED_PENDING
            ))
        );
        assert_eq!(
            ledger.resident_state(column),
            Some(ResidentColumnState::STAGED_PENDING)
        );
    }

    #[test]
    fn cancelled_loads_restart_with_a_newer_ticket() {
        let column = ChunkColumn::new(2, 3);
        let mut ledger = ColumnResidencyLedger::new(Entity::PLACEHOLDER);
        let first = ledger.begin_load(column, 3).unwrap();

        assert!(ledger.cancel_load(first));
        let second = ledger.begin_load(column, 4).unwrap();

        assert!(second.version() > first.version());
        assert_eq!(second.view_revision(), 4);
        assert!(!ledger.accept_load(first));
        assert!(ledger.accept_load(second));
    }

    #[test]
    fn transient_failures_back_off_exponentially_per_column() {
        let failed_column = ChunkColumn::new(0, 0);
        let independent_column = ChunkColumn::new(1, 0);
        let mut ledger = ColumnResidencyLedger::new(Entity::PLACEHOLDER);
        let first = ledger.begin_load(failed_column, 5).unwrap();
        assert!(ledger.fail_load(first, transient_error()));

        let Some(ColumnResidency::Failed(failure)) = ledger.state(failed_column) else {
            panic!("failed load must enter backoff");
        };
        assert!(failure.error().is_transient());
        assert_eq!(failure.attempts(), 1);
        assert_eq!(failure.retry_after_updates(), Some(30));
        assert!(ledger.begin_load(failed_column, 5).is_none());
        assert!(ledger.begin_load(independent_column, 5).is_some());

        for _ in 0..30 {
            ledger.tick_backoffs();
        }
        let second = ledger.begin_load(failed_column, 5).unwrap();
        assert!(second.version() > first.version());
        assert!(ledger.fail_load(second, transient_error()));
        let Some(ColumnResidency::Failed(failure)) = ledger.state(failed_column) else {
            panic!("second failure must enter backoff");
        };
        assert_eq!(failure.attempts(), 2);
        assert_eq!(failure.retry_after_updates(), Some(60));
    }

    #[test]
    fn exit_and_reentry_invalidate_load_tickets_and_failure_history() {
        let column = ChunkColumn::new(-1, 5);
        let mut ledger = ColumnResidencyLedger::new(Entity::PLACEHOLDER);
        let old = ledger.begin_load(column, 10).unwrap();

        assert_eq!(ledger.mark_undesired(column), None);
        assert!(!ledger.accept_load(old));
        assert!(!ledger.mark_desired(column));
        let failed = ledger.begin_load(column, 11).unwrap();
        assert!(ledger.fail_load(failed, permanent_error()));
        assert!(ledger.begin_load(column, 11).is_none());

        ledger.mark_undesired(column);
        assert!(!ledger.mark_desired(column));
        let fresh = ledger.begin_load(column, 12).unwrap();
        let Some(ColumnResidency::Loading { attempt, .. }) = ledger.state(column) else {
            panic!("re-entered column must start loading");
        };
        assert_eq!(*attempt, 1);
        assert!(fresh.version() > failed.version());
        assert_eq!(fresh.view_revision(), 12);
        assert!(!ledger.accept_load(failed));
    }

    #[test]
    fn reentry_cancels_eviction_and_invalidates_its_ticket() {
        let column = ChunkColumn::new(6, -2);
        let mut ledger = ColumnResidencyLedger::new(Entity::PLACEHOLDER);
        let load = load_resident(&mut ledger, column, 1);
        assert_eq!(ledger.finish_lighting(column), Some(ColumnLightRevision(1)));
        assert!(ledger.publish(column));
        let resident = ledger.resident_state(column).unwrap();
        let first = ledger.mark_undesired(column).unwrap();
        assert!(first.version() > load.version());
        assert_eq!(first.owner(), ledger.owner());
        assert_eq!(first.column(), column);
        assert_eq!(ledger.eviction_ticket(column), Some(first));
        assert_eq!(ledger.loading_ticket(column), None);
        assert_eq!(
            ledger.state(column),
            Some(&ColumnResidency::Evicting {
                ticket: first,
                resident,
            })
        );

        assert!(ledger.mark_desired(column));
        assert_eq!(
            ledger.state(column),
            Some(&ColumnResidency::Resident(resident))
        );
        assert!(!ledger.commit_eviction(first));

        let second = ledger.mark_undesired(column).unwrap();
        assert!(second.version() > first.version());
        assert!(ledger.commit_eviction(second));
        assert!(ledger.state(column).is_none());
    }

    #[test]
    fn repeated_undesired_marking_returns_the_active_eviction_ticket() {
        let column = ChunkColumn::new(3, 4);
        let mut ledger = ColumnResidencyLedger::new(Entity::PLACEHOLDER);
        load_resident(&mut ledger, column, 1);

        let first = ledger.mark_undesired(column).unwrap();
        let second = ledger.mark_undesired(column).unwrap();

        assert_eq!(first, second);
        assert!(ledger.commit_eviction(first));
    }

    #[test]
    fn lighting_and_exposure_transition_independently() {
        let column = ChunkColumn::new(-7, 12);
        let mut ledger = ColumnResidencyLedger::new(Entity::PLACEHOLDER);
        load_resident(&mut ledger, column, 1);

        let pending = ledger.resident_state(column).unwrap();
        assert_eq!(pending.lighting(), ColumnLighting::Pending);
        assert_eq!(pending.exposure(), ColumnExposure::Staged);
        assert_eq!(pending.light_revision(), ColumnLightRevision::INITIAL);
        assert_eq!(pending.light_revision(), ColumnLightRevision(0));
        assert!(pending.is_light_pending());
        assert!(pending.is_staged());
        assert!(!ledger.publish(column));
        assert!(!ledger.mark_light_pending(column));

        let first_revision = ledger.finish_lighting(column).unwrap();
        assert_eq!(first_revision, ColumnLightRevision(1));
        let staged_lit = ledger.resident_state(column).unwrap();
        assert_eq!(staged_lit.lighting(), ColumnLighting::Lit);
        assert_eq!(staged_lit.light_revision(), first_revision);
        assert!(staged_lit.is_lit());
        assert!(staged_lit.is_staged());
        assert_eq!(ledger.finish_lighting(column), None);

        assert!(ledger.publish(column));
        let published_lit = ledger.resident_state(column).unwrap();
        assert!(published_lit.is_published());
        assert!(published_lit.is_lit());
        assert!(!ledger.publish(column));

        assert!(ledger.mark_light_pending(column));
        let published_pending = ledger.resident_state(column).unwrap();
        assert!(published_pending.is_published());
        assert!(published_pending.is_light_pending());
        assert_eq!(published_pending.light_revision(), first_revision);

        let second_revision = ledger.finish_lighting(column).unwrap();
        assert_eq!(second_revision, ColumnLightRevision(2));
        assert!(ledger.resident_state(column).unwrap().is_published());
        assert!(ledger.unpublish(column));
        let staged_lit = ledger.resident_state(column).unwrap();
        assert!(staged_lit.is_staged());
        assert_eq!(staged_lit.light_revision(), second_revision);
        assert!(!ledger.unpublish(column));
    }

    #[test]
    fn eviction_freezes_resident_phase_until_reentry_or_commit() {
        let column = ChunkColumn::new(8, 3);
        let mut ledger = ColumnResidencyLedger::new(Entity::PLACEHOLDER);
        load_resident(&mut ledger, column, 2);
        assert!(ledger.finish_lighting(column).is_some());
        let resident = ledger.resident_state(column).unwrap();
        let ticket = ledger.mark_undesired(column).unwrap();

        assert_eq!(ledger.resident_state(column), None);
        assert!(!ledger.mark_light_pending(column));
        assert_eq!(ledger.finish_lighting(column), None);
        assert!(!ledger.publish(column));
        assert!(!ledger.unpublish(column));
        assert_eq!(
            ledger.state(column),
            Some(&ColumnResidency::Evicting { ticket, resident })
        );

        assert!(ledger.mark_desired(column));
        assert_eq!(ledger.resident_state(column), Some(resident));
        let next = ledger.mark_undesired(column).unwrap();
        assert_ne!(next, ticket);
        assert!(ledger.commit_eviction(next));
        assert!(ledger.state(column).is_none());
    }

    #[test]
    fn stats_report_each_residency_state() {
        let mut ledger = ColumnResidencyLedger::new(Entity::PLACEHOLDER);
        let loading = ChunkColumn::new(0, 0);
        let resident = ChunkColumn::new(1, 0);
        let evicting = ChunkColumn::new(2, 0);
        let failed = ChunkColumn::new(3, 0);

        let loading_ticket = ledger.begin_load(loading, 1).unwrap();
        assert!(ledger.accept_load(loading_ticket));
        load_resident(&mut ledger, resident, 1);
        load_resident(&mut ledger, evicting, 1);
        ledger.mark_undesired(evicting);
        let failed_ticket = ledger.begin_load(failed, 1).unwrap();
        ledger.fail_load(failed_ticket, transient_error());
        for _ in 0..30 {
            ledger.tick_backoffs();
        }

        assert_eq!(
            ledger.stats(),
            ColumnResidencyStats {
                loading: 1,
                accepted_loads: 1,
                resident: 1,
                evicting: 1,
                failed: 1,
                retryable_failures: 1,
            }
        );
        assert_eq!(ledger.states().len(), 4);
    }

    #[test]
    fn retry_delay_is_bounded_exponential() {
        assert_eq!(retry_delay_for_attempt(1), 30);
        assert_eq!(retry_delay_for_attempt(2), 60);
        assert_eq!(retry_delay_for_attempt(6), 600);
        assert_eq!(retry_delay_for_attempt(20), 600);
    }
}
