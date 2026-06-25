use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::block::{BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, PADDED_CHUNK_SIZE, PADDED_CHUNK_VOLUME,
    block_mesh_flags, padded_chunk_index,
};

use super::super::chunk_neighbor_offsets;

/// One entry in the pre-computed full-cube-cell list.
///
/// `yz_bit`, `xz_bit`, `xy_bit` are the plane bit indices (0..324).
/// `x`, `y`, `z` are padded coordinates (0..18) used for plane-indexing
/// and the `is_center` check.
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub(crate) struct FullCubeCell {
    pub yz_bit: u16,
    pub xz_bit: u16,
    pub xy_bit: u16,
    pub x: u8,
    pub y: u8,
    pub z: u8,
}

pub struct ChunkMeshBlocks {
    pub(crate) blocks: Box<[u16; PADDED_CHUNK_VOLUME]>,
    pub(crate) fluid_levels: Box<[u8; PADDED_CHUNK_VOLUME]>,
    pub(crate) center_rendered_blocks: u16,
    pub(crate) center_full_cube_blocks: u16,
    pub(crate) neighbor_face_shells_full_cube: bool,
    pub(crate) full_cube_cells: Box<[FullCubeCell]>,
}

impl ChunkMeshBlocks {
    #[cfg(test)]
    pub fn from_chunk(chunk: &Chunk) -> Self {
        let mut blocks = Self::empty();
        blocks.copy_center_chunk(chunk);
        compute_full_cube_cells_impl(&blocks.blocks, &mut blocks.full_cube_cells);
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
            full_cube_cells: Vec::new().into_boxed_slice(),
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
        for x in source_range_for_neighbor_axis(offset.x) {
            for y in source_range_for_neighbor_axis(offset.y) {
                for z in source_range_for_neighbor_axis(offset.z) {
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

        let x = (x + 1) as usize;
        let y = (y + 1) as usize;
        let z = (z + 1) as usize;
        self.blocks[padded_chunk_index(x, y, z)] = cell;
    }

    fn set_fluid_level(&mut self, x: i32, y: i32, z: i32, level: u8) {
        debug_assert!(is_in_padded_chunk(x));
        debug_assert!(is_in_padded_chunk(y));
        debug_assert!(is_in_padded_chunk(z));

        let x = (x + 1) as usize;
        let y = (y + 1) as usize;
        let z = (z + 1) as usize;
        self.fluid_levels[padded_chunk_index(x, y, z)] = level;
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

    #[allow(dead_code)]
    pub(crate) fn compute_full_cube_cells(&mut self) {
        compute_full_cube_cells_impl(&self.blocks, &mut self.full_cube_cells);
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

        for x in source_range_for_neighbor_axis(offset.x) {
            for y in source_range_for_neighbor_axis(offset.y) {
                for z in source_range_for_neighbor_axis(offset.z) {
                    if chunk.hot_meta_xyz(x, y, z).mesh_flags & BLOCK_FLAG_FULL_CUBE == 0 {
                        return false;
                    }
                }
            }
        }
    }

    true
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

/// Pre-compute the packed (x, y, z) coordinates of every full-cube cell in
/// the padded neighbourhood.  `BinaryFaceMasks::from_padded` iterates this
/// list instead of scanning all 5 832 cells, skipping the 93%+ of cells that
/// are air or non-full-cube.
pub(crate) fn compute_full_cube_cells_impl(
    cells: &[u16; PADDED_CHUNK_VOLUME],
    out: &mut Box<[FullCubeCell]>,
) {
    let pad = PADDED_CHUNK_SIZE;
    let mut entries: Vec<FullCubeCell> = Vec::with_capacity(4096);
    for x in 0..pad {
        for y in 0..pad {
            for z in 0..pad {
                let idx = padded_chunk_index(x, y, z);
                let flags = block_mesh_flags(cells[idx]);
                if flags & (BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE)
                    == (BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE)
                {
                    entries.push(FullCubeCell {
                        yz_bit: (y * pad + z) as u16,
                        xz_bit: (x * pad + z) as u16,
                        xy_bit: (x * pad + y) as u16,
                        x: x as u8,
                        y: y as u8,
                        z: z as u8,
                    });
                }
            }
        }
    }
    *out = entries.into_boxed_slice();
}
