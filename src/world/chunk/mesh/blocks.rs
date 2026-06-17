use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::block::BlockType;

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, FULL_CUBE_BITMASK_SIZE, PADDED_CHUNK_VOLUME,
    padded_chunk_index,
};

use super::super::chunk_neighbor_offsets;

pub struct ChunkMeshBlocks {
    pub(crate) blocks: Box<[BlockType; PADDED_CHUNK_VOLUME]>,
    pub(crate) full_cube_bitmask: Box<[u32; FULL_CUBE_BITMASK_SIZE]>,
    pub(crate) center_rendered_blocks: u16,
    pub(crate) center_full_cube_blocks: u16,
}

impl ChunkMeshBlocks {
    pub fn from_chunk(chunk: &Chunk) -> Self {
        let mut blocks = Self::empty();
        blocks.copy_center_chunk(chunk);
        blocks
    }

    pub fn from_chunks(center_pos: IVec3, chunks: &HashMap<IVec3, &Chunk>) -> Self {
        let mut blocks = Self::empty();

        for offset in std::iter::once(IVec3::ZERO).chain(chunk_neighbor_offsets()) {
            let Some(chunk) = chunks.get(&(center_pos + offset)).copied() else {
                continue;
            };

            if offset == IVec3::ZERO {
                blocks.copy_center_chunk(chunk);
            } else {
                blocks.copy_neighbor_chunk_region(offset, chunk);
            }
        }

        blocks
    }

    fn empty() -> Self {
        Self {
            blocks: Box::new([BlockType::Air; PADDED_CHUNK_VOLUME]),
            full_cube_bitmask: Box::new([0u32; FULL_CUBE_BITMASK_SIZE]),
            center_rendered_blocks: 0,
            center_full_cube_blocks: 0,
        }
    }

    #[inline(always)]
    pub(crate) fn is_full_cube_at(&self, index: usize) -> bool {
        self.full_cube_bit_at(index) != 0
    }

    #[inline(always)]
    pub(crate) fn full_cube_bit_at(&self, index: usize) -> u32 {
        debug_assert!(index < PADDED_CHUNK_VOLUME);
        let word = unsafe { *self.full_cube_bitmask.get_unchecked(index >> 5) };
        (word >> (index & 31)) & 1
    }

    fn copy_center_chunk(&mut self, chunk: &Chunk) {
        let mut rendered_blocks = 0;
        let mut full_cube_blocks = 0;

        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    let block = chunk.blocks[x][z][y];
                    self.set_block(x as i32, y as i32, z as i32, block);
                    rendered_blocks += block.is_rendered() as u16;
                    full_cube_blocks += block.is_full_cube() as u16;
                }
            }
        }

        self.center_rendered_blocks = rendered_blocks;
        self.center_full_cube_blocks = full_cube_blocks;
    }

    fn copy_neighbor_chunk_region(&mut self, offset: IVec3, chunk: &Chunk) {
        for x in source_range_for_neighbor_axis(offset.x) {
            for y in source_range_for_neighbor_axis(offset.y) {
                for z in source_range_for_neighbor_axis(offset.z) {
                    self.set_block(
                        x as i32 + offset.x * CHUNK_ISIZE,
                        y as i32 + offset.y * CHUNK_ISIZE,
                        z as i32 + offset.z * CHUNK_ISIZE,
                        chunk.blocks[x][z][y],
                    );
                }
            }
        }
    }

    fn set_block(&mut self, x: i32, y: i32, z: i32, block: BlockType) {
        debug_assert!(is_in_padded_chunk(x));
        debug_assert!(is_in_padded_chunk(y));
        debug_assert!(is_in_padded_chunk(z));

        let x = (x + 1) as usize;
        let y = (y + 1) as usize;
        let z = (z + 1) as usize;
        let index = padded_chunk_index(x, y, z);
        self.blocks[index] = block;
        if block.is_full_cube() {
            self.full_cube_bitmask[index >> 5] |= 1u32 << (index & 31);
        }
    }

    pub(crate) fn can_skip_mesh(&self) -> bool {
        self.center_rendered_blocks == 0
            || (self.center_is_all_full_cube() && self.neighbor_face_shells_are_full_cube())
    }

    pub(crate) fn center_is_all_full_cube(&self) -> bool {
        self.center_full_cube_blocks as usize == CHUNK_VOLUME
    }

    fn neighbor_face_shells_are_full_cube(&self) -> bool {
        for y in 1..=CHUNK_SIZE {
            for z in 1..=CHUNK_SIZE {
                if !self.is_full_cube_at(padded_chunk_index(0, y, z))
                    || !self.is_full_cube_at(padded_chunk_index(CHUNK_SIZE + 1, y, z))
                {
                    return false;
                }
            }
        }

        for x in 1..=CHUNK_SIZE {
            for z in 1..=CHUNK_SIZE {
                if !self.is_full_cube_at(padded_chunk_index(x, 0, z))
                    || !self.is_full_cube_at(padded_chunk_index(x, CHUNK_SIZE + 1, z))
                {
                    return false;
                }
            }
        }

        for x in 1..=CHUNK_SIZE {
            for y in 1..=CHUNK_SIZE {
                if !self.is_full_cube_at(padded_chunk_index(x, y, 0))
                    || !self.is_full_cube_at(padded_chunk_index(x, y, CHUNK_SIZE + 1))
                {
                    return false;
                }
            }
        }

        true
    }
}

fn source_range_for_neighbor_axis(delta: i32) -> std::ops::Range<usize> {
    match delta {
        -1 => CHUNK_SIZE - 1..CHUNK_SIZE,
        0 => 0..CHUNK_SIZE,
        1 => 0..1,
        _ => unreachable!("invalid neighbor offset"),
    }
}

fn is_in_padded_chunk(value: i32) -> bool {
    (-1..=CHUNK_ISIZE).contains(&value)
}
