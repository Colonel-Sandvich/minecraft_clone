use bevy::prelude::*;

use crate::block::{BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED, HotBlockStateMeta};

use super::state::{CellDelta, ChunkCell};

#[derive(Resource, Debug, Default)]
pub struct ChunkPerfCounters {
    pub mesh_rebuilds: usize,
    pub light_rebuild_targets: usize,
    pub light_uploads: usize,
}

impl ChunkPerfCounters {
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChunkBlockCounts {
    pub rendered: u16,
    pub full_cubes: u16,
    pub translucent: u16,
}

impl ChunkBlockCounts {
    pub fn apply_delta(&mut self, delta: CellDelta) {
        let (old_rendered, old_full, old_trans) = cell_counts(delta.old);
        let (new_rendered, new_full, new_trans) = cell_counts(delta.new);
        self.rendered = self
            .rendered
            .wrapping_add(new_rendered)
            .wrapping_sub(old_rendered);
        self.full_cubes = self
            .full_cubes
            .wrapping_add(new_full)
            .wrapping_sub(old_full);
        self.translucent = self
            .translucent
            .wrapping_add(new_trans)
            .wrapping_sub(old_trans);
    }
}

fn cell_counts(cell: ChunkCell) -> (u16, u16, u16) {
    meta_counts(cell.hot_meta())
}

pub(super) fn meta_counts(meta: HotBlockStateMeta) -> (u16, u16, u16) {
    let rendered = (meta.mesh_flags & BLOCK_FLAG_RENDERED != 0) as u16;
    let full_cubes = (meta.mesh_flags & BLOCK_FLAG_FULL_CUBE != 0) as u16;
    (rendered, full_cubes, rendered.saturating_sub(full_cubes))
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkPosition(pub IVec3);

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsSave;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsMeshRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsLightUpload;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsColliderRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsLightRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkHasActiveFluids;
