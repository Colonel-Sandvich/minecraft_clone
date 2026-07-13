use std::time::Duration;

use bevy::prelude::*;

use crate::block::{BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED, BLOCK_FLAG_TRANSLUCENT};

use super::{
    coords::ChunkPos,
    state::{CellDelta, ChunkCell},
};

#[derive(Resource, Debug, Default)]
pub struct ChunkPerfCounters {
    pub column_loads: usize,
    pub column_load_worker_elapsed: Duration,
    pub column_load_max_worker_elapsed: Duration,
    pub column_load_queue_elapsed: Duration,
    pub column_load_max_queue_elapsed: Duration,
    pub column_load_pickup_lag: Duration,
    pub column_load_max_pickup_lag: Duration,
    pub column_load_latency: Duration,
    pub column_load_max_latency: Duration,
    pub mesh_rebuilds: usize,
    pub mesh_rebuild_runs: usize,
    pub mesh_rebuild_elapsed: Duration,
    pub mesh_rebuild_max_elapsed: Duration,
    pub mesh_context_elapsed: Duration,
    pub mesh_context_max_elapsed: Duration,
    pub mesh_build_elapsed: Duration,
    pub mesh_build_max_elapsed: Duration,
    pub mesh_apply_elapsed: Duration,
    pub mesh_apply_max_elapsed: Duration,
    pub light_rebuild_targets: usize,
    pub light_patch_runs: usize,
    pub light_patch_calculation_chunks: usize,
    pub light_patch_max_calculation_chunks: usize,
    pub light_patch_scratch_chunks: usize,
    pub light_patch_accepted_results: usize,
    pub light_patch_committed_columns: usize,
    pub light_patch_elapsed: Duration,
    pub light_patch_max_elapsed: Duration,
    pub light_patch_solve_elapsed: Duration,
    pub light_patch_prepare_elapsed: Duration,
    pub light_patch_plan_elapsed: Duration,
    pub light_patch_max_plan_elapsed: Duration,
    pub light_patch_snapshot_elapsed: Duration,
    pub light_patch_max_snapshot_elapsed: Duration,
    pub light_patch_collect_elapsed: Duration,
    pub light_patch_max_collect_elapsed: Duration,
    pub light_patch_queue_elapsed: Duration,
    pub light_patch_max_queue_elapsed: Duration,
    pub light_patch_pickup_lag: Duration,
    pub light_patch_max_pickup_lag: Duration,
    pub light_patch_latency: Duration,
    pub light_patch_max_latency: Duration,
    pub light_patch_stale_results: usize,
    pub light_patch_cancelled: usize,
    pub light_uploads: usize,
}

impl ChunkPerfCounters {
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}

/// Exact semantic totals used by chunk consumers and incremental mutation.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChunkContentCounts {
    pub rendered: u16,
    pub full_cubes: u16,
    pub solid: u16,
    pub translucent: u16,
    pub fluids: u16,
}

impl ChunkContentCounts {
    pub fn apply_delta(&mut self, delta: CellDelta) {
        if delta.old == delta.new {
            return;
        }

        let old = CellCounts::from_cell(delta.old);
        let new = CellCounts::from_cell(delta.new);

        *self = Self {
            rendered: apply_count_delta(self.rendered, old.rendered, new.rendered, "rendered"),
            full_cubes: apply_count_delta(
                self.full_cubes,
                old.full_cubes,
                new.full_cubes,
                "full-cube",
            ),
            solid: apply_count_delta(self.solid, old.solid, new.solid, "solid"),
            translucent: apply_count_delta(
                self.translucent,
                old.translucent,
                new.translucent,
                "translucent",
            ),
            fluids: apply_count_delta(self.fluids, old.fluids, new.fluids, "fluid"),
        };
    }
}

#[derive(Debug, Clone, Copy)]
struct CellCounts {
    rendered: u16,
    full_cubes: u16,
    solid: u16,
    translucent: u16,
    fluids: u16,
}

impl CellCounts {
    fn from_cell(cell: ChunkCell) -> Self {
        let flags = cell.hot_meta().mesh_flags;
        Self {
            rendered: (flags & BLOCK_FLAG_RENDERED != 0) as u16,
            full_cubes: (flags & BLOCK_FLAG_FULL_CUBE != 0) as u16,
            solid: cell.is_solid() as u16,
            translucent: (flags & BLOCK_FLAG_TRANSLUCENT != 0) as u16,
            fluids: cell.is_fluid() as u16,
        }
    }
}

fn apply_count_delta(count: u16, old: u16, new: u16, name: &str) -> u16 {
    count
        .checked_sub(old)
        .unwrap_or_else(|| panic!("{name} content count underflow"))
        .checked_add(new)
        .unwrap_or_else(|| panic!("{name} content count overflow"))
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkPosition(ChunkPos);

impl ChunkPosition {
    pub const fn from_chunk_pos(position: ChunkPos) -> Self {
        Self(position)
    }

    pub const fn chunk_pos(self) -> ChunkPos {
        self.0
    }

    pub const fn as_ivec3(self) -> IVec3 {
        self.0.as_ivec3()
    }
}

impl From<ChunkPos> for ChunkPosition {
    fn from(position: ChunkPos) -> Self {
        Self::from_chunk_pos(position)
    }
}

impl From<IVec3> for ChunkPosition {
    fn from(position: IVec3) -> Self {
        Self::from_chunk_pos(ChunkPos::from_ivec3(position))
    }
}

impl From<ChunkPosition> for ChunkPos {
    fn from(position: ChunkPosition) -> Self {
        position.chunk_pos()
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsSave;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsColliderRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsLightRebuild;

/// Marks a chunk that has pending fluid simulation work.
///
/// A settled chunk can contain fluid cells without carrying this marker.
#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsFluidStep;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;
    use crate::world::chunk::Chunk;

    fn add(counts: &mut ChunkContentCounts, cell: ChunkCell) {
        counts.apply_delta(CellDelta {
            old: ChunkCell::EMPTY,
            new: cell,
        });
    }

    #[test]
    fn chunk_position_preserves_typed_and_vector_inputs() {
        let position = ChunkPos::new(-7, 3, 11);

        assert_eq!(ChunkPosition::from(position).chunk_pos(), position);
        assert_eq!(
            ChunkPosition::from(position.as_ivec3()).chunk_pos(),
            position
        );
        assert_eq!(
            ChunkPosition::from(position).as_ivec3(),
            position.as_ivec3()
        );
    }

    #[test]
    fn content_counts_track_independent_cell_properties() {
        let mut counts = ChunkContentCounts::default();

        add(&mut counts, BlockType::Stone.into());
        add(&mut counts, BlockType::OakLeaves.into());
        add(&mut counts, BlockType::Ice.into());
        add(&mut counts, ChunkCell::water_source());

        assert_eq!(
            counts,
            ChunkContentCounts {
                rendered: 4,
                full_cubes: 1,
                solid: 3,
                translucent: 2,
                fluids: 1,
            }
        );
    }

    #[test]
    fn replacing_a_fluid_with_a_full_cube_updates_every_affected_count() {
        let mut counts = ChunkContentCounts::default();
        add(&mut counts, ChunkCell::water_source());

        counts.apply_delta(CellDelta {
            old: ChunkCell::water_source(),
            new: BlockType::Stone.into(),
        });

        assert_eq!(
            counts,
            ChunkContentCounts {
                rendered: 1,
                full_cubes: 1,
                solid: 1,
                translucent: 0,
                fluids: 0,
            }
        );
    }

    #[test]
    fn incremental_counts_match_a_full_recount_after_mixed_mutations() {
        let mut chunk = Chunk::default();
        let mut counts = ChunkContentCounts::default();

        for (pos, cell) in [
            (uvec3(1, 2, 3), BlockType::Stone.into()),
            (uvec3(4, 5, 6), BlockType::OakLeaves.into()),
            (uvec3(7, 8, 9), ChunkCell::water_source()),
            (uvec3(1, 2, 3), BlockType::Glass.into()),
            (uvec3(4, 5, 6), ChunkCell::EMPTY),
            (uvec3(7, 8, 9), BlockType::Ice.into()),
        ] {
            counts.apply_delta(chunk.set_cell(pos, cell));
            assert_eq!(counts, chunk.compute_content_counts());
        }
    }

    #[test]
    #[should_panic(expected = "rendered content count underflow")]
    fn inconsistent_delta_panics_instead_of_wrapping() {
        ChunkContentCounts::default().apply_delta(CellDelta {
            old: BlockType::Stone.into(),
            new: ChunkCell::EMPTY,
        });
    }

    #[test]
    #[should_panic(expected = "rendered content count overflow")]
    fn count_overflow_panics_instead_of_wrapping() {
        let mut counts = ChunkContentCounts {
            rendered: u16::MAX,
            ..Default::default()
        };
        add(&mut counts, BlockType::Stone.into());
    }
}
