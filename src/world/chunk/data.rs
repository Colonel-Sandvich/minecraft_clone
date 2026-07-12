use bevy::prelude::*;

use crate::block::BlockType;

use super::{
    components::ChunkContentCounts,
    coords::{CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, LocalBlockPos, chunk_linear_index},
    state::{
        AIR_CELL_STATE_ID, CELL_REGISTRY, CellDelta, CellRegistry, CellStateId, ChunkCell,
        FluidState, HotCellMeta,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub state: CellStateId,
    pub hot: HotCellMeta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkPalette {
    entries: Vec<PaletteEntry>,
}

impl Default for ChunkPalette {
    fn default() -> Self {
        Self {
            entries: vec![PaletteEntry {
                state: AIR_CELL_STATE_ID,
                hot: HotCellMeta::AIR,
            }],
        }
    }
}

impl ChunkPalette {
    pub fn entries(&self) -> &[PaletteEntry] {
        &self.entries
    }

    #[inline(always)]
    fn entry(&self, index: u32) -> PaletteEntry {
        self.entries[index as usize]
    }

    #[inline(always)]
    fn cell(&self, index: u32) -> ChunkCell {
        ChunkCell::from_state_id(self.entry(index).state).expect("invalid state in chunk palette")
    }

    fn index_for_state(&self, state: CellStateId) -> Option<u32> {
        self.entries
            .iter()
            .position(|entry| entry.state == state)
            .map(|index| index as u32)
    }

    fn get_or_insert_cell(&mut self, cell: ChunkCell) -> u32 {
        let state = cell.state_id();
        if let Some(index) = self.index_for_state(state) {
            return index;
        }

        let index = self.entries.len() as u32;
        self.entries.push(PaletteEntry {
            state,
            hot: CELL_REGISTRY
                .hot_meta(state)
                .expect("state id from chunk cell must be valid"),
        });
        index
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellStorage {
    U8(Box<[u8; CHUNK_VOLUME]>),
    U16(Box<[u16; CHUNK_VOLUME]>),
    U32(Box<[u32; CHUNK_VOLUME]>),
}

impl Default for CellStorage {
    fn default() -> Self {
        Self::U8(Box::new([0; CHUNK_VOLUME]))
    }
}

impl CellStorage {
    #[inline(always)]
    pub fn get_linear(&self, index: usize) -> u32 {
        match self {
            Self::U8(cells) => cells[index] as u32,
            Self::U16(cells) => cells[index] as u32,
            Self::U32(cells) => cells[index],
        }
    }

    #[inline(always)]
    fn set_linear(&mut self, index: usize, palette_index: u32) {
        match self {
            Self::U8(cells) => cells[index] = palette_index as u8,
            Self::U16(cells) => cells[index] = palette_index as u16,
            Self::U32(cells) => cells[index] = palette_index,
        }
    }

    #[inline(always)]
    fn max_index(&self) -> u32 {
        match self {
            Self::U8(_) => u8::MAX as u32,
            Self::U16(_) => u16::MAX as u32,
            Self::U32(_) => u32::MAX,
        }
    }

    fn promote_for_index(&mut self, palette_index: u32) {
        if palette_index <= self.max_index() {
            return;
        }

        if palette_index <= u16::MAX as u32 {
            let mut promoted = Box::new([0u16; CHUNK_VOLUME]);
            for (index, cell) in promoted.iter_mut().enumerate() {
                *cell = self.get_linear(index) as u16;
            }
            *self = Self::U16(promoted);
        } else {
            let mut promoted = Box::new([0u32; CHUNK_VOLUME]);
            for (index, cell) in promoted.iter_mut().enumerate() {
                *cell = self.get_linear(index);
            }
            *self = Self::U32(promoted);
        }
    }

    fn fill(&mut self, palette_index: u32) {
        self.promote_for_index(palette_index);
        match self {
            Self::U8(cells) => cells.fill(palette_index as u8),
            Self::U16(cells) => cells.fill(palette_index as u16),
            Self::U32(cells) => cells.fill(palette_index),
        }
    }
}

#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkRevision(u64);

impl ChunkRevision {
    pub const INITIAL: Self = Self(0);

    pub const fn get(self) -> u64 {
        self.0
    }

    fn advance(&mut self) {
        self.0 = self
            .0
            .checked_add(1)
            .expect("chunk content revision overflowed");
    }
}

#[derive(Component, Debug, Clone)]
pub struct Chunk {
    palette: ChunkPalette,
    cells: CellStorage,
    content_revision: ChunkRevision,
}

impl PartialEq for Chunk {
    fn eq(&self, other: &Self) -> bool {
        (0..CHUNK_VOLUME).all(|index| self.state_id_linear(index) == other.state_id_linear(index))
    }
}

impl Eq for Chunk {}

impl Default for Chunk {
    fn default() -> Self {
        Self {
            palette: ChunkPalette::default(),
            cells: CellStorage::default(),
            content_revision: ChunkRevision::INITIAL,
        }
    }
}

impl Chunk {
    pub fn filled(cell: ChunkCell) -> Self {
        let mut chunk = Self::default();
        chunk.fill_untracked(cell);
        chunk
    }

    pub fn from_cell_fn(mut cell_at: impl FnMut(usize, usize, usize) -> ChunkCell) -> Self {
        let mut chunk = Self::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let index = chunk_linear_index(x, y, z);
                    chunk.write_cell_linear(index, cell_at(x, y, z));
                }
            }
        }
        chunk
    }

    pub fn palette(&self) -> &ChunkPalette {
        &self.palette
    }

    pub fn cell_storage(&self) -> &CellStorage {
        &self.cells
    }

    pub const fn content_revision(&self) -> ChunkRevision {
        self.content_revision
    }

    #[inline(always)]
    pub fn get_cell(&self, pos: UVec3) -> ChunkCell {
        self.cell_xyz(pos.x as usize, pos.y as usize, pos.z as usize)
    }

    #[inline(always)]
    pub fn cell(&self, local: LocalBlockPos) -> ChunkCell {
        self.cell_linear(local.index().as_usize())
    }

    #[inline(always)]
    pub fn cell_xyz(&self, x: usize, y: usize, z: usize) -> ChunkCell {
        self.cell_linear(chunk_linear_index(x, y, z))
    }

    #[inline(always)]
    pub fn cell_linear(&self, index: usize) -> ChunkCell {
        self.palette.cell(self.cells.get_linear(index))
    }

    #[inline(always)]
    pub fn palette_index(&self, pos: UVec3) -> u32 {
        self.palette_index_xyz(pos.x as usize, pos.y as usize, pos.z as usize)
    }

    #[inline(always)]
    pub fn palette_index_xyz(&self, x: usize, y: usize, z: usize) -> u32 {
        self.cells.get_linear(chunk_linear_index(x, y, z))
    }

    #[inline(always)]
    pub fn palette_index_linear(&self, index: usize) -> u32 {
        self.cells.get_linear(index)
    }

    #[inline(always)]
    pub fn state_id(&self, pos: UVec3) -> CellStateId {
        self.state_id_linear(chunk_linear_index(
            pos.x as usize,
            pos.y as usize,
            pos.z as usize,
        ))
    }

    #[inline(always)]
    pub fn state_id_linear(&self, index: usize) -> CellStateId {
        self.palette.entry(self.cells.get_linear(index)).state
    }

    #[inline(always)]
    pub fn hot_meta(&self, pos: UVec3) -> HotCellMeta {
        self.hot_meta_xyz(pos.x as usize, pos.y as usize, pos.z as usize)
    }

    #[inline(always)]
    pub fn hot_meta_xyz(&self, x: usize, y: usize, z: usize) -> HotCellMeta {
        self.hot_meta_linear(chunk_linear_index(x, y, z))
    }

    #[inline(always)]
    pub fn hot_meta_linear(&self, index: usize) -> HotCellMeta {
        self.palette.entry(self.cells.get_linear(index)).hot
    }

    #[inline(always)]
    pub fn get_block(&self, pos: UVec3) -> Option<BlockType> {
        self.get_cell(pos).as_block()
    }

    #[inline(always)]
    pub fn set_cell(&mut self, pos: UVec3, cell: ChunkCell) -> CellDelta {
        self.set_cell_xyz(pos.x as usize, pos.y as usize, pos.z as usize, cell)
    }

    #[inline(always)]
    pub fn set_cell_xyz(&mut self, x: usize, y: usize, z: usize, cell: ChunkCell) -> CellDelta {
        self.set_cell_linear(chunk_linear_index(x, y, z), cell)
    }

    pub fn set_cell_linear(&mut self, index: usize, cell: ChunkCell) -> CellDelta {
        let old = self.cell_linear(index);
        if old != cell {
            self.write_cell_linear(index, cell);
            self.content_revision.advance();
        }
        let new = cell;
        CellDelta { old, new }
    }

    pub fn set_state(
        &mut self,
        pos: UVec3,
        state: CellStateId,
        registry: &CellRegistry,
    ) -> Option<CellDelta> {
        registry.cell(state).map(|cell| self.set_cell(pos, cell))
    }

    pub(super) fn write_cell_linear(&mut self, index: usize, cell: ChunkCell) -> ChunkCell {
        let palette_index = self.palette.get_or_insert_cell(cell);
        self.cells.promote_for_index(palette_index);
        self.cells.set_linear(index, palette_index);
        cell
    }

    pub fn set_block(&mut self, pos: UVec3, block: BlockType) -> CellDelta {
        self.set_cell(pos, block.into())
    }

    pub fn set_empty(&mut self, pos: UVec3) -> CellDelta {
        self.set_cell(pos, ChunkCell::EMPTY)
    }

    pub fn set_fluid(&mut self, pos: UVec3, fluid: FluidState) -> CellDelta {
        self.set_cell(pos, ChunkCell::fluid(fluid))
    }

    pub fn fill(&mut self, cell: ChunkCell) {
        if (0..CHUNK_VOLUME).all(|index| self.cell_linear(index) == cell) {
            return;
        }

        self.fill_untracked(cell);
        self.content_revision.advance();
    }

    fn fill_untracked(&mut self, cell: ChunkCell) {
        let palette_index = self.palette.get_or_insert_cell(cell);
        self.cells.fill(palette_index);
    }

    pub fn get_i(&self, x: i32, y: i32, z: i32) -> Option<ChunkCell> {
        let outside = |a: i32| !(0..CHUNK_ISIZE).contains(&a);
        if outside(x) || outside(y) || outside(z) {
            return None;
        }

        Some(self.cell_xyz(x as usize, y as usize, z as usize))
    }

    pub fn place_cell(&mut self, pos: UVec3, cell: ChunkCell) -> Option<CellDelta> {
        if !cell.is_rendered() {
            return None;
        }

        let old = self.get_cell(pos);
        if !old.can_be_replaced_by_placement() {
            return None;
        };

        Some(self.set_cell(pos, cell))
    }

    pub fn place_block(&mut self, pos: UVec3, block: BlockType) -> Option<CellDelta> {
        if !block.is_placeable() {
            return None;
        }

        self.place_cell(pos, block.into())
    }

    pub fn break_block(&mut self, pos: UVec3) -> Option<CellDelta> {
        if !self.get_cell(pos).is_solid() {
            return None;
        };

        Some(self.set_empty(pos))
    }

    pub fn compute_content_counts(&self) -> ChunkContentCounts {
        let mut counts = ChunkContentCounts::default();
        for index in 0..CHUNK_VOLUME {
            counts.apply_delta(CellDelta {
                old: ChunkCell::EMPTY,
                new: self.cell_linear(index),
            });
        }
        counts
    }

    pub fn iter(&self) -> ChunkCellIter<'_> {
        ChunkCellIter {
            chunk: self,
            index: 0,
        }
    }

    pub(super) fn storage_palette(&self) -> Vec<ChunkCell> {
        let mut palette = Vec::new();
        for (cell, _) in self.iter() {
            if !palette.contains(&cell) {
                palette.push(cell);
            }
        }
        palette
    }

    pub(crate) fn to_cell_buffer(&self) -> [ChunkCell; CHUNK_VOLUME] {
        std::array::from_fn(|index| self.cell_linear(index))
    }
}

pub struct ChunkCellIter<'a> {
    chunk: &'a Chunk,
    index: usize,
}

impl Iterator for ChunkCellIter<'_> {
    type Item = (ChunkCell, LocalBlockPos);

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= CHUNK_VOLUME {
            return None;
        }

        let index = self.index;
        self.index += 1;
        let local = super::coords::ChunkIndex::try_from_usize(index)
            .expect("chunk iterator index must be in bounds")
            .local();

        Some((self.chunk.cell_linear(index), local))
    }
}
