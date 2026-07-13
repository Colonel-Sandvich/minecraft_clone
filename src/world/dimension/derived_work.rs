use std::collections::VecDeque;

use bevy::{platform::collections::HashMap, prelude::Entity};

use crate::world::chunk::{ChunkInvalidationEffects, ChunkPos};

/// A disposable derived-data consumer for a chunk.
///
/// Saving is deliberately absent: durable dirty state must survive queue
/// cancellation, dimension deactivation, and frame budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum ChunkDerivedWorkKind {
    MeshRebuild = 1 << 0,
    ColliderRebuild = 1 << 1,
    LightRebuild = 1 << 2,
    FluidStep = 1 << 3,
    RenderLightUpload = 1 << 4,
}

impl ChunkDerivedWorkKind {
    const ALL: [Self; 5] = [
        Self::MeshRebuild,
        Self::ColliderRebuild,
        Self::LightRebuild,
        Self::FluidStep,
        Self::RenderLightUpload,
    ];

    const fn bit(self) -> u8 {
        self as u8
    }
}

/// A coalesced set of disposable chunk effects.
///
/// This is intentionally distinct from [`ChunkInvalidationEffects`]. That
/// type can include a durable save obligation, while this type cannot express
/// one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ChunkDerivedEffects(u8);

impl ChunkDerivedEffects {
    pub(crate) const NONE: Self = Self(0);
    const ALL: Self = Self((1 << ChunkDerivedWorkKind::ALL.len()) - 1);

    pub(crate) const fn only(kind: ChunkDerivedWorkKind) -> Self {
        Self(kind.bit())
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) const fn contains(self, kind: ChunkDerivedWorkKind) -> bool {
        self.0 & kind.bit() != 0
    }

    pub(crate) const fn with(self, kind: ChunkDerivedWorkKind) -> Self {
        Self(self.0 | kind.bit())
    }

    const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    fn insert(&mut self, effects: Self) {
        self.0 |= effects.0;
    }

    fn remove(&mut self, effects: Self) {
        self.0 &= !effects.0;
    }
}

impl From<ChunkDerivedWorkKind> for ChunkDerivedEffects {
    fn from(kind: ChunkDerivedWorkKind) -> Self {
        Self::only(kind)
    }
}

impl From<ChunkInvalidationEffects> for ChunkDerivedEffects {
    fn from(effects: ChunkInvalidationEffects) -> Self {
        let mut derived = Self::NONE;
        if effects.needs_mesh_rebuild() {
            derived = derived.with(ChunkDerivedWorkKind::MeshRebuild);
        }
        if effects.needs_collider_rebuild() {
            derived = derived.with(ChunkDerivedWorkKind::ColliderRebuild);
        }
        if effects.needs_light_rebuild() {
            derived = derived.with(ChunkDerivedWorkKind::LightRebuild);
        }
        if effects.needs_fluid_step() {
            derived = derived.with(ChunkDerivedWorkKind::FluidStep);
        }
        if effects.needs_render_light_upload() {
            derived = derived.with(ChunkDerivedWorkKind::RenderLightUpload);
        }
        derived
    }
}

/// Disposable work addressed to one exact chunk incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChunkDerivedWork {
    position: ChunkPos,
    expected_entity: Entity,
    effects: ChunkDerivedEffects,
}

impl ChunkDerivedWork {
    pub(crate) const fn position(self) -> ChunkPos {
        self.position
    }

    pub(crate) const fn expected_entity(self) -> Entity {
        self.expected_entity
    }

    pub(crate) const fn effects(self) -> ChunkDerivedEffects {
        self.effects
    }
}

/// Dimension-owned, coalescing work for disposable chunk derivatives.
///
/// Positions retain FIFO admission order. Re-recording work for the same
/// entity merges effects in place. Recording a different entity at the same
/// position discards the stale incarnation's effects and admits the replacement
/// at the back of the queue.
#[derive(Debug, Default)]
pub(crate) struct DimensionDerivedWork {
    entries: HashMap<ChunkPos, DerivedWorkEntry>,
    order: VecDeque<AdmissionToken>,
    next_sequence: u64,
    tombstones: usize,
}

#[derive(Debug, Clone, Copy)]
struct DerivedWorkEntry {
    expected_entity: Entity,
    effects: ChunkDerivedEffects,
    sequence: u64,
}

impl DerivedWorkEntry {
    const fn as_work(self, position: ChunkPos) -> ChunkDerivedWork {
        ChunkDerivedWork {
            position,
            expected_entity: self.expected_entity,
            effects: self.effects,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AdmissionToken {
    position: ChunkPos,
    sequence: u64,
}

impl DimensionDerivedWork {
    const MIN_TOMBSTONES_TO_COMPACT: usize = 32;

    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn chunk_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn effect_count(&self, kind: ChunkDerivedWorkKind) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.effects.contains(kind))
            .count()
    }

    pub(crate) fn get(&self, position: ChunkPos) -> Option<ChunkDerivedWork> {
        self.entries
            .get(&position)
            .copied()
            .map(|entry| entry.as_work(position))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = ChunkDerivedWork> + '_ {
        self.order.iter().filter_map(|token| {
            let entry = self.entries.get(&token.position)?;
            (entry.sequence == token.sequence).then(|| entry.as_work(token.position))
        })
    }

    pub(crate) fn pending(
        &self,
        kind: ChunkDerivedWorkKind,
    ) -> impl Iterator<Item = ChunkDerivedWork> + '_ {
        self.iter().filter(move |work| work.effects.contains(kind))
    }

    /// Records work and reports whether the pending state changed.
    pub(crate) fn record(
        &mut self,
        position: ChunkPos,
        expected_entity: Entity,
        effects: ChunkDerivedEffects,
    ) -> bool {
        let Some(existing) = self.entries.get_mut(&position) else {
            if effects.is_empty() {
                return false;
            }
            self.admit(position, expected_entity, effects);
            return true;
        };

        if existing.expected_entity == expected_entity {
            let previous = existing.effects;
            existing.effects.insert(effects);
            return existing.effects != previous;
        }

        self.entries.remove(&position);
        self.tombstones += 1;
        if !effects.is_empty() {
            self.admit(position, expected_entity, effects);
        }
        self.compact_if_needed();
        true
    }

    pub(crate) fn record_invalidations(
        &mut self,
        position: ChunkPos,
        expected_entity: Entity,
        effects: ChunkInvalidationEffects,
    ) -> bool {
        self.record(position, expected_entity, effects.into())
    }

    /// Takes only the requested effects when the entity incarnation matches.
    ///
    /// Unrequested effects remain queued. A stale consumer cannot take work
    /// belonging to a replacement entity at the same position.
    pub(crate) fn take(
        &mut self,
        position: ChunkPos,
        expected_entity: Entity,
        requested: ChunkDerivedEffects,
    ) -> Option<ChunkDerivedWork> {
        let work = self.take_without_compacting(position, expected_entity, requested);
        self.compact_if_needed();
        work
    }

    fn take_without_compacting(
        &mut self,
        position: ChunkPos,
        expected_entity: Entity,
        requested: ChunkDerivedEffects,
    ) -> Option<ChunkDerivedWork> {
        let entry = self.entries.get_mut(&position)?;
        if entry.expected_entity != expected_entity {
            return None;
        }

        let taken = entry.effects.intersection(requested);
        if taken.is_empty() {
            return None;
        }
        entry.effects.remove(taken);

        let work = ChunkDerivedWork {
            position,
            expected_entity,
            effects: taken,
        };
        if entry.effects.is_empty() {
            self.entries.remove(&position);
            self.tombstones += 1;
        }
        Some(work)
    }

    /// Takes at most `limit` pending chunks for one consumer in FIFO order.
    pub(crate) fn take_up_to(
        &mut self,
        kind: ChunkDerivedWorkKind,
        limit: usize,
    ) -> Vec<ChunkDerivedWork> {
        let selected = self.pending(kind).take(limit).collect::<Vec<_>>();
        let requested = ChunkDerivedEffects::only(kind);

        let taken = selected
            .into_iter()
            .map(|work| {
                self.take_without_compacting(work.position, work.expected_entity, requested)
                    .expect("selected derived work must remain pending until it is taken")
            })
            .collect();
        self.compact_if_needed();
        taken
    }

    /// Removes all work for the matching incarnation.
    pub(crate) fn remove(&mut self, position: ChunkPos, expected_entity: Entity) -> bool {
        self.take(position, expected_entity, ChunkDerivedEffects::ALL)
            .is_some()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.next_sequence = 0;
        self.tombstones = 0;
    }

    fn admit(&mut self, position: ChunkPos, expected_entity: Entity, effects: ChunkDerivedEffects) {
        debug_assert!(!effects.is_empty());
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("derived-work admission sequence exhausted");
        let previous = self.entries.insert(
            position,
            DerivedWorkEntry {
                expected_entity,
                effects,
                sequence,
            },
        );
        debug_assert!(previous.is_none());
        self.order.push_back(AdmissionToken { position, sequence });
    }

    fn compact_if_needed(&mut self) {
        debug_assert_eq!(self.order.len(), self.entries.len() + self.tombstones);
        if self.entries.is_empty() {
            self.order.clear();
            self.tombstones = 0;
            return;
        }

        let live_tokens = self.order.len() - self.tombstones;
        if self.tombstones < Self::MIN_TOMBSTONES_TO_COMPACT || self.tombstones < live_tokens {
            return;
        }

        self.order.retain(|token| {
            self.entries
                .get(&token.position)
                .is_some_and(|entry| entry.sequence == token.sequence)
        });
        self.tombstones = 0;
        debug_assert_eq!(self.order.len(), self.entries.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block::BlockType,
        world::chunk::{
            CellDelta, ChunkCell, ChunkContentCounts, ChunkInvalidationPlan, LocalBlockPos,
        },
    };

    const MESH: ChunkDerivedWorkKind = ChunkDerivedWorkKind::MeshRebuild;
    const COLLIDER: ChunkDerivedWorkKind = ChunkDerivedWorkKind::ColliderRebuild;
    const LIGHT: ChunkDerivedWorkKind = ChunkDerivedWorkKind::LightRebuild;
    const FLUID: ChunkDerivedWorkKind = ChunkDerivedWorkKind::FluidStep;
    const RENDER_LIGHT: ChunkDerivedWorkKind = ChunkDerivedWorkKind::RenderLightUpload;

    fn entity(bits: u64) -> Entity {
        Entity::from_bits(bits)
    }

    fn effects(kinds: &[ChunkDerivedWorkKind]) -> ChunkDerivedEffects {
        kinds
            .iter()
            .fold(ChunkDerivedEffects::NONE, |effects, &kind| {
                effects.with(kind)
            })
    }

    #[test]
    fn invalidation_conversion_keeps_only_disposable_effects() {
        let position = ChunkPos::new(2, 1, -4);
        let mut changed = ChunkInvalidationPlan::new();
        changed.record_cell_delta(
            position,
            LocalBlockPos::new(3, 5, 7),
            CellDelta {
                old: ChunkCell::EMPTY,
                new: BlockType::Stone.into(),
            },
        );
        let derived = ChunkDerivedEffects::from(changed.effects_for(position).unwrap());

        assert!(derived.contains(MESH));
        assert!(derived.contains(COLLIDER));
        assert!(derived.contains(LIGHT));
        assert!(derived.contains(FLUID));
        assert!(!derived.contains(RENDER_LIGHT));

        let mut published = ChunkInvalidationPlan::new();
        published.record_chunk_published(
            position,
            ChunkContentCounts {
                rendered: 1,
                solid: 1,
                fluids: 1,
                ..ChunkContentCounts::default()
            },
        );
        let derived = ChunkDerivedEffects::from(published.effects_for(position).unwrap());

        assert_eq!(derived, effects(&[MESH, COLLIDER, FLUID, RENDER_LIGHT]));
        assert_eq!(
            ChunkDerivedEffects::from(ChunkInvalidationEffects::NONE),
            ChunkDerivedEffects::NONE
        );
    }

    #[test]
    fn same_entity_coalesces_effects_without_duplicating_the_chunk() {
        let mut queue = DimensionDerivedWork::new();
        let position = ChunkPos::new(1, 2, 3);
        let expected = entity(7);

        assert!(queue.record(position, expected, MESH.into()));
        assert!(queue.record(position, expected, COLLIDER.into()));
        assert!(!queue.record(position, expected, MESH.into()));

        assert_eq!(queue.chunk_count(), 1);
        assert_eq!(queue.effect_count(MESH), 1);
        assert_eq!(queue.effect_count(COLLIDER), 1);
        assert_eq!(
            queue.iter().collect::<Vec<_>>(),
            vec![ChunkDerivedWork {
                position,
                expected_entity: expected,
                effects: effects(&[MESH, COLLIDER]),
            }]
        );
    }

    #[test]
    fn replacement_entity_drops_stale_effects_and_reenters_at_the_back() {
        let mut queue = DimensionDerivedWork::new();
        let replaced_position = ChunkPos::new(1, 0, 0);
        let other_position = ChunkPos::new(2, 0, 0);
        let old = entity(9);
        let replacement = entity((1_u64 << 32) | 9);

        queue.record(replaced_position, old, MESH.into());
        queue.record(other_position, entity(10), LIGHT.into());
        queue.record(replaced_position, replacement, COLLIDER.into());

        let work = queue.iter().collect::<Vec<_>>();
        assert_eq!(work[0].position(), other_position);
        assert_eq!(work[1].position(), replaced_position);
        assert_eq!(work[1].expected_entity(), replacement);
        assert_eq!(work[1].effects(), COLLIDER.into());
        assert_eq!(queue.effect_count(MESH), 0);
    }

    #[test]
    fn empty_replacement_clears_stale_work_but_empty_same_entity_is_a_noop() {
        let mut queue = DimensionDerivedWork::new();
        let position = ChunkPos::new(1, 0, 0);
        let old = entity(11);
        let replacement = entity((1_u64 << 32) | 11);

        queue.record(position, old, MESH.into());
        assert!(!queue.record(position, old, ChunkDerivedEffects::NONE));
        assert!(queue.record(position, replacement, ChunkDerivedEffects::NONE));
        assert!(queue.is_empty());
    }

    #[test]
    fn selective_take_preserves_unrequested_effects() {
        let mut queue = DimensionDerivedWork::new();
        let position = ChunkPos::new(4, 3, 2);
        let expected = entity(12);
        queue.record(position, expected, effects(&[MESH, COLLIDER, LIGHT]));

        let taken = queue
            .take(position, expected, effects(&[MESH, LIGHT]))
            .unwrap();

        assert_eq!(taken.effects(), effects(&[MESH, LIGHT]));
        assert_eq!(queue.get(position).unwrap().effects(), COLLIDER.into());
        assert_eq!(queue.chunk_count(), 1);
    }

    #[test]
    fn stale_take_and_remove_cannot_consume_replacement_work() {
        let mut queue = DimensionDerivedWork::new();
        let position = ChunkPos::new(5, 4, 3);
        let old = entity(13);
        let replacement = entity((1_u64 << 32) | 13);
        queue.record(position, replacement, effects(&[MESH, COLLIDER]));

        assert_eq!(queue.take(position, old, MESH.into()), None);
        assert!(!queue.remove(position, old));
        assert_eq!(queue.get(position).unwrap().expected_entity(), replacement);
        assert_eq!(
            queue.get(position).unwrap().effects(),
            effects(&[MESH, COLLIDER])
        );
    }

    #[test]
    fn bounded_take_is_fifo_per_kind_and_leaves_other_work_queued() {
        let mut queue = DimensionDerivedWork::new();
        let positions = [
            ChunkPos::new(0, 0, 0),
            ChunkPos::new(1, 0, 0),
            ChunkPos::new(2, 0, 0),
            ChunkPos::new(3, 0, 0),
        ];
        queue.record(positions[0], entity(20), effects(&[MESH, COLLIDER]));
        queue.record(positions[1], entity(21), COLLIDER.into());
        queue.record(positions[2], entity(22), MESH.into());
        queue.record(positions[3], entity(23), MESH.into());

        let taken = queue.take_up_to(MESH, 2);

        assert_eq!(
            taken.iter().map(|work| work.position()).collect::<Vec<_>>(),
            vec![positions[0], positions[2]]
        );
        assert!(taken.iter().all(|work| work.effects() == MESH.into()));
        assert_eq!(queue.effect_count(MESH), 1);
        assert_eq!(queue.effect_count(COLLIDER), 2);
        assert_eq!(
            queue
                .pending(COLLIDER)
                .map(|work| work.position())
                .collect::<Vec<_>>(),
            vec![positions[0], positions[1]]
        );
    }

    #[test]
    fn replacement_churn_preserves_fifo_and_compacts_tombstones_amortized() {
        let mut queue = DimensionDerivedWork::new();
        let replaced = ChunkPos::new(0, 0, 0);
        let waiting = ChunkPos::new(1, 0, 0);
        let later = ChunkPos::new(2, 0, 0);
        queue.record(replaced, entity(50), MESH.into());
        queue.record(waiting, entity(51), MESH.into());

        let mut replacement = entity(50);
        for bits in 100..200 {
            replacement = entity(bits);
            queue.record(replaced, replacement, MESH.into());
        }

        assert_eq!(
            queue.iter().map(|work| work.position()).collect::<Vec<_>>(),
            vec![waiting, replaced]
        );
        assert!(
            queue.order.len()
                < queue.entries.len() + DimensionDerivedWork::MIN_TOMBSTONES_TO_COMPACT
        );

        assert_eq!(queue.take_up_to(MESH, 1)[0].position(), waiting);
        queue.record(later, entity(52), MESH.into());
        assert_eq!(
            queue
                .take_up_to(MESH, usize::MAX)
                .into_iter()
                .map(|work| (work.position(), work.expected_entity()))
                .collect::<Vec<_>>(),
            vec![(replaced, replacement), (later, entity(52))]
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn remove_clear_and_counts_keep_queue_indexes_consistent() {
        let mut queue = DimensionDerivedWork::new();
        let first = ChunkPos::new(-2, 1, 4);
        let second = ChunkPos::new(-1, 1, 4);
        let first_entity = entity(30);
        let second_entity = entity(31);
        queue.record(first, first_entity, MESH.into());
        queue.record(second, second_entity, effects(&[MESH, FLUID]));

        assert!(queue.remove(first, first_entity));
        assert!(!queue.remove(first, first_entity));
        assert_eq!(queue.chunk_count(), 1);
        assert_eq!(queue.effect_count(MESH), 1);
        assert_eq!(queue.effect_count(FLUID), 1);

        queue.clear();
        assert!(queue.is_empty());
        assert_eq!(queue.chunk_count(), 0);
        assert_eq!(queue.iter().count(), 0);
    }

    #[test]
    fn zero_budget_does_not_consume_work() {
        let mut queue = DimensionDerivedWork::new();
        let position = ChunkPos::new(8, 0, -8);
        queue.record(position, entity(40), MESH.into());

        assert!(queue.take_up_to(MESH, 0).is_empty());
        assert!(queue.get(position).is_some());
    }

    #[test]
    fn record_invalidations_accepts_no_durable_save_bit() {
        let mut plan = ChunkInvalidationPlan::new();
        let position = ChunkPos::new(9, 0, -9);
        plan.record_cell_delta(
            position,
            LocalBlockPos::new(1, 1, 1),
            CellDelta {
                old: ChunkCell::EMPTY,
                new: BlockType::Dirt.into(),
            },
        );

        let mut queue = DimensionDerivedWork::new();
        queue.record_invalidations(position, entity(41), plan.effects_for(position).unwrap());

        let queued = queue.get(position).unwrap();
        assert_eq!(queued.effects(), effects(&[MESH, COLLIDER, LIGHT, FLUID]));
    }
}
