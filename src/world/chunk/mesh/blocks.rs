use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::block::{BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED};
use crate::quad::Direction;

pub(crate) use super::super::neighborhood::{
    PADDED_CHUNK_LAYER_SIZE, PADDED_CHUNK_SIZE, PADDED_CHUNK_VOLUME, padded_chunk_index,
};
use super::super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, chunk_neighbor_offsets,
    neighborhood::{NeighborOffset, PaddedChunkIndex, PaddedChunkOffset},
};

pub(crate) const DIRECTION_COUNT: usize = Direction::COUNT;
pub(crate) const DIRECTION_INDEX_OFFSETS: [isize; DIRECTION_COUNT] = [
    PaddedChunkOffset::for_direction(Direction::Left).as_isize(),
    PaddedChunkOffset::for_direction(Direction::Right).as_isize(),
    PaddedChunkOffset::for_direction(Direction::Down).as_isize(),
    PaddedChunkOffset::for_direction(Direction::Up).as_isize(),
    PaddedChunkOffset::for_direction(Direction::Forward).as_isize(),
    PaddedChunkOffset::for_direction(Direction::Backward).as_isize(),
];

const _: () = {
    assert!(Direction::Left.index() == 0);
    assert!(Direction::Right.index() == 1);
    assert!(Direction::Down.index() == 2);
    assert!(Direction::Up.index() == 3);
    assert!(Direction::Forward.index() == 4);
    assert!(Direction::Backward.index() == 5);
};

pub struct ChunkMeshBlocks {
    pub(crate) blocks: Box<[u16; PADDED_CHUNK_VOLUME]>,
    pub(crate) fluid_levels: Box<[u8; PADDED_CHUNK_VOLUME]>,
    pub(crate) center_rendered_blocks: u16,
    pub(crate) center_full_cube_blocks: u16,
    pub(crate) neighbor_face_shells_full_cube: bool,
}

impl ChunkMeshBlocks {
    #[cfg(test)]
    pub fn from_chunk(chunk: &Chunk) -> Self {
        let mut blocks = Self::empty();
        blocks.copy_center_chunk(chunk);
        blocks
    }

    pub fn from_chunks(center_pos: IVec3, chunks: &HashMap<IVec3, &Chunk>) -> Self {
        let mut blocks = Self::empty();

        let Some(center) = chunks.get(&center_pos).copied() else {
            return blocks;
        };

        blocks.copy_center_chunk(center);
        if blocks.center_rendered_blocks == 0 {
            return blocks;
        }

        if blocks.center_is_all_full_cube() && neighbor_face_shells_full_cube(center_pos, chunks) {
            blocks.neighbor_face_shells_full_cube = true;
            return blocks;
        }

        for offset in chunk_neighbor_offsets() {
            let Some(chunk) = chunks.get(&(center_pos + offset)).copied() else {
                continue;
            };

            blocks.copy_neighbor_chunk_region(offset, chunk);
        }

        blocks
    }

    fn empty() -> Self {
        Self {
            blocks: Box::new([0u16; PADDED_CHUNK_VOLUME]),
            fluid_levels: Box::new([0; PADDED_CHUNK_VOLUME]),
            center_rendered_blocks: 0,
            center_full_cube_blocks: 0,
            neighbor_face_shells_full_cube: false,
        }
    }

    fn copy_center_chunk(&mut self, chunk: &Chunk) {
        let mut rendered_blocks = 0;
        let mut full_cube_blocks = 0;

        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    let meta = chunk.hot_meta_xyz(x, y, z);
                    self.set_cell_kind(x as i32, y as i32, z as i32, meta.render_id);
                    self.set_fluid_level(x as i32, y as i32, z as i32, meta.fluid_level);
                    rendered_blocks += (meta.mesh_flags & BLOCK_FLAG_RENDERED != 0) as u16;
                    full_cube_blocks += (meta.mesh_flags & BLOCK_FLAG_FULL_CUBE != 0) as u16;
                }
            }
        }

        self.center_rendered_blocks = rendered_blocks;
        self.center_full_cube_blocks = full_cube_blocks;
    }

    fn copy_neighbor_chunk_region(&mut self, offset: IVec3, chunk: &Chunk) {
        for x in NeighborOffset::source_axis_range(offset.x) {
            for y in NeighborOffset::source_axis_range(offset.y) {
                for z in NeighborOffset::source_axis_range(offset.z) {
                    let meta = chunk.hot_meta_xyz(x, y, z);
                    self.set_cell_kind(
                        x as i32 + offset.x * CHUNK_ISIZE,
                        y as i32 + offset.y * CHUNK_ISIZE,
                        z as i32 + offset.z * CHUNK_ISIZE,
                        meta.render_id,
                    );
                    self.set_fluid_level(
                        x as i32 + offset.x * CHUNK_ISIZE,
                        y as i32 + offset.y * CHUNK_ISIZE,
                        z as i32 + offset.z * CHUNK_ISIZE,
                        meta.fluid_level,
                    );
                }
            }
        }
    }

    fn set_cell_kind(&mut self, x: i32, y: i32, z: i32, cell: u16) {
        debug_assert!(is_in_padded_chunk(x));
        debug_assert!(is_in_padded_chunk(y));
        debug_assert!(is_in_padded_chunk(z));

        let index = PaddedChunkIndex::from_relative(IVec3::new(x, y, z))
            .expect("mesh coordinate must fit the padded chunk");
        self.blocks[index.as_usize()] = cell;
    }

    fn set_fluid_level(&mut self, x: i32, y: i32, z: i32, level: u8) {
        debug_assert!(is_in_padded_chunk(x));
        debug_assert!(is_in_padded_chunk(y));
        debug_assert!(is_in_padded_chunk(z));

        let index = PaddedChunkIndex::from_relative(IVec3::new(x, y, z))
            .expect("mesh coordinate must fit the padded chunk");
        self.fluid_levels[index.as_usize()] = level;
    }

    #[inline(always)]
    pub(crate) fn get_fluid_level(&self, padded_index: usize) -> u8 {
        unsafe { *self.fluid_levels.get_unchecked(padded_index) }
    }

    pub(crate) fn can_skip_mesh(&self) -> bool {
        self.center_rendered_blocks == 0
            || (self.center_is_all_full_cube() && self.neighbor_face_shells_full_cube)
    }

    pub(crate) fn has_non_full_cube_rendered(&self) -> bool {
        self.center_rendered_blocks > self.center_full_cube_blocks
    }

    pub(crate) fn center_is_all_full_cube(&self) -> bool {
        self.center_full_cube_blocks as usize == CHUNK_VOLUME
    }
}

const FACE_NEIGHBOR_OFFSETS: [IVec3; 6] = [
    IVec3::NEG_X,
    IVec3::X,
    IVec3::NEG_Y,
    IVec3::Y,
    IVec3::NEG_Z,
    IVec3::Z,
];

fn neighbor_face_shells_full_cube(center_pos: IVec3, chunks: &HashMap<IVec3, &Chunk>) -> bool {
    for offset in FACE_NEIGHBOR_OFFSETS {
        let Some(chunk) = chunks.get(&(center_pos + offset)).copied() else {
            return false;
        };

        for x in NeighborOffset::source_axis_range(offset.x) {
            for y in NeighborOffset::source_axis_range(offset.y) {
                for z in NeighborOffset::source_axis_range(offset.z) {
                    if chunk.hot_meta_xyz(x, y, z).mesh_flags & BLOCK_FLAG_FULL_CUBE == 0 {
                        return false;
                    }
                }
            }
        }
    }

    true
}

fn is_in_padded_chunk(value: i32) -> bool {
    (-1..=CHUNK_ISIZE).contains(&value)
}
