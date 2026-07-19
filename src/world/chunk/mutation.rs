use crate::item::Item;

use super::{
    components::ChunkContentCounts,
    coords::{ChunkPos, LocalBlockPos},
    data::Chunk,
    invalidation::ChunkInvalidationPlan,
    state::{CellDelta, ChunkCell, FluidState},
};

/// The tracked write boundary for a loaded chunk.
///
/// Generation and decoding can write directly to `Chunk`; runtime systems use
/// this editor so storage, semantic counts, and derived work cannot diverge.
pub struct ChunkEditor<'a> {
    position: ChunkPos,
    chunk: &'a mut Chunk,
    counts: &'a mut ChunkContentCounts,
    invalidations: &'a mut ChunkInvalidationPlan,
}

impl<'a> ChunkEditor<'a> {
    pub fn new(
        position: ChunkPos,
        chunk: &'a mut Chunk,
        counts: &'a mut ChunkContentCounts,
        invalidations: &'a mut ChunkInvalidationPlan,
    ) -> Self {
        Self {
            position,
            chunk,
            counts,
            invalidations,
        }
    }

    pub const fn position(&self) -> ChunkPos {
        self.position
    }

    pub fn cell(&self, local: LocalBlockPos) -> ChunkCell {
        self.chunk.cell(local)
    }

    /// Replaces a cell, returning `None` when its semantic state is unchanged.
    pub fn set_cell(&mut self, local: LocalBlockPos, new: ChunkCell) -> Option<CellDelta> {
        let old = self.cell(local);
        if old == new {
            return None;
        }

        Some(self.commit(local, CellDelta { old, new }))
    }

    pub fn set_block(&mut self, local: LocalBlockPos, block: Item) -> Option<CellDelta> {
        self.set_cell(local, ChunkCell::block(block))
    }

    pub fn set_fluid(&mut self, local: LocalBlockPos, fluid: FluidState) -> Option<CellDelta> {
        self.set_cell(local, ChunkCell::fluid(fluid))
    }

    pub fn set_empty(&mut self, local: LocalBlockPos) -> Option<CellDelta> {
        self.set_cell(local, ChunkCell::EMPTY)
    }

    pub fn place_cell(&mut self, local: LocalBlockPos, cell: ChunkCell) -> Option<CellDelta> {
        if !cell.is_rendered() {
            return None;
        }

        let old = self.cell(local);
        if old == cell || !old.can_be_replaced_by_placement() {
            return None;
        }

        Some(self.commit(local, CellDelta { old, new: cell }))
    }

    pub fn place_block(&mut self, local: LocalBlockPos, block: Item) -> Option<CellDelta> {
        if !block.is_placeable() {
            return None;
        }

        self.place_cell(local, ChunkCell::block(block))
    }

    pub fn break_block(&mut self, local: LocalBlockPos) -> Option<CellDelta> {
        let old = self.cell(local);
        if !old.is_solid() {
            return None;
        }

        Some(self.commit(
            local,
            CellDelta {
                old,
                new: ChunkCell::EMPTY,
            },
        ))
    }

    fn commit(&mut self, local: LocalBlockPos, delta: CellDelta) -> CellDelta {
        self.counts.apply_delta(delta);
        let written = self.chunk.set_cell(local.as_uvec3(), delta.new);
        debug_assert_eq!(written, delta);
        self.invalidations
            .record_cell_delta(self.position, local, delta);
        delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_no_ops_leave_storage_counts_and_work_untouched() {
        let mut chunk = Chunk::default();
        let mut counts = ChunkContentCounts::default();
        let mut invalidations = ChunkInvalidationPlan::new();
        let palette_len = chunk.palette().entries().len();

        {
            let mut editor = ChunkEditor::new(
                ChunkPos::new(-7, 3, 11),
                &mut chunk,
                &mut counts,
                &mut invalidations,
            );
            assert_eq!(editor.set_cell(LocalBlockPos::ZERO, ChunkCell::EMPTY), None);
            assert_eq!(
                editor.place_cell(LocalBlockPos::ZERO, ChunkCell::EMPTY),
                None
            );
            assert_eq!(editor.break_block(LocalBlockPos::ZERO), None);
        }

        assert_eq!(chunk.palette().entries().len(), palette_len);
        assert_eq!(counts, ChunkContentCounts::default());
        assert!(invalidations.is_empty());
    }

    #[test]
    fn mixed_edits_keep_incremental_counts_equal_to_a_full_recount() {
        let position = ChunkPos::new(-2, 4, 9);
        let local = LocalBlockPos::ZERO;
        let mut chunk = Chunk::default();
        let mut counts = ChunkContentCounts::default();
        let mut invalidations = ChunkInvalidationPlan::new();

        {
            let mut editor =
                ChunkEditor::new(position, &mut chunk, &mut counts, &mut invalidations);
            assert!(editor.set_block(local, Item::Stone).is_some());
            assert!(editor.set_block(local, Item::Glass).is_some());
            assert!(editor.set_cell(local, ChunkCell::water_source()).is_some());
            assert_eq!(editor.set_cell(local, ChunkCell::water_source()), None);
        }

        assert_eq!(counts, chunk.compute_content_counts());
        assert_eq!(counts.fluids, 1);
        assert_eq!(invalidations.chunk_count(), 8);
        let own = invalidations.effects_for(position).unwrap();
        assert!(own.needs_save());
        assert!(own.needs_mesh_rebuild());
        assert!(own.needs_collider_rebuild());
        assert!(own.needs_light_rebuild());
        assert!(own.needs_fluid_step());
    }

    #[test]
    fn placement_and_breaking_rules_are_enforced_at_the_write_boundary() {
        let local = LocalBlockPos::new(3, 5, 7);
        let mut chunk = Chunk::default();
        let mut counts = ChunkContentCounts::default();
        let mut invalidations = ChunkInvalidationPlan::new();

        {
            let mut editor =
                ChunkEditor::new(ChunkPos::ZERO, &mut chunk, &mut counts, &mut invalidations);
            assert!(
                editor
                    .place_cell(local, ChunkCell::water_source())
                    .is_some()
            );
            assert_eq!(editor.break_block(local), None);
            assert!(editor.place_block(local, Item::Stone).is_some());
            assert_eq!(editor.place_block(local, Item::Dirt), None);
            assert!(editor.break_block(local).is_some());
        }

        assert_eq!(chunk.get_cell(local.as_uvec3()), ChunkCell::EMPTY);
        assert_eq!(counts, ChunkContentCounts::default());
    }
}
