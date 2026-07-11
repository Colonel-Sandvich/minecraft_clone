//! Ambient-occlusion sampling and compact per-face AO keys.

use crate::block::BLOCK_FLAG_FULL_CUBE;

use super::super::blocks::{ChunkMeshBlocks, DIRECTION_COUNT, PADDED_CHUNK_VOLUME};
use super::visibility::block_mesh_flags;

const VERTEX_AO: [u32; 8] = [3, 2, 2, 0, 2, 1, 1, 0];
pub(crate) const FACE_AO_SAMPLE_COUNT: usize = 8;

#[derive(Clone, Copy)]
pub(crate) enum FaceAoOrder {
    Ab,
    Ba,
}

pub(crate) const FACE_AO_ORDERS: [FaceAoOrder; DIRECTION_COUNT] = [
    FaceAoOrder::Ab,
    FaceAoOrder::Ab,
    FaceAoOrder::Ba,
    FaceAoOrder::Ab,
    FaceAoOrder::Ba,
    FaceAoOrder::Ba,
];

pub(crate) const FACE_AO_SAMPLE_OFFSETS: [[isize; FACE_AO_SAMPLE_COUNT]; DIRECTION_COUNT] = [
    [-325, 323, 17, -19, -307, -343, 341, 305],
    [-323, 325, -17, 19, -341, -305, 307, 343],
    [-325, -323, -306, -342, -307, -343, -305, -341],
    [323, 325, 342, 306, 341, 305, 343, 307],
    [-19, -17, -342, 306, -343, 305, -341, 307],
    [19, 17, -306, 342, -305, 343, -307, 341],
];

#[inline(always)]
pub(crate) fn face_ao_key_from_indices(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    side_index: usize,
) -> u32 {
    let [a0, a1, b0, b1, c00, c01, c10, c11] = FACE_AO_SAMPLE_OFFSETS[side_index];

    face_ao_key_from_sample_bits(
        FACE_AO_ORDERS[side_index],
        [
            block_occludes_ambient_light_bit(blocks, padded_index, a0),
            block_occludes_ambient_light_bit(blocks, padded_index, a1),
            block_occludes_ambient_light_bit(blocks, padded_index, b0),
            block_occludes_ambient_light_bit(blocks, padded_index, b1),
            block_occludes_ambient_light_bit(blocks, padded_index, c00),
            block_occludes_ambient_light_bit(blocks, padded_index, c01),
            block_occludes_ambient_light_bit(blocks, padded_index, c10),
            block_occludes_ambient_light_bit(blocks, padded_index, c11),
        ],
    )
}

#[inline(always)]
pub(crate) fn face_ao_key_from_sample_bits(
    order: FaceAoOrder,
    samples: [u32; FACE_AO_SAMPLE_COUNT],
) -> u32 {
    let [a0, a1, b0, b1, c00, c01, c10, c11] = samples;
    match order {
        FaceAoOrder::Ab => {
            vertex_ao_key(a0, b0, c00)
                | (vertex_ao_key(a0, b1, c01) << 2)
                | (vertex_ao_key(a1, b0, c10) << 4)
                | (vertex_ao_key(a1, b1, c11) << 6)
        }
        FaceAoOrder::Ba => {
            vertex_ao_key(a0, b0, c00)
                | (vertex_ao_key(a1, b0, c10) << 2)
                | (vertex_ao_key(a0, b1, c01) << 4)
                | (vertex_ao_key(a1, b1, c11) << 6)
        }
    }
}

#[inline(always)]
#[cfg(test)]
pub(crate) fn face_ao_from_indices(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    side_index: usize,
) -> [u8; 4] {
    let key = face_ao_key_from_indices(blocks, padded_index, side_index);
    [
        (key & 0x3) as u8,
        ((key >> 2) & 0x3) as u8,
        ((key >> 4) & 0x3) as u8,
        ((key >> 6) & 0x3) as u8,
    ]
}

#[inline(always)]
pub(crate) fn vertex_ao_key(side_a: u32, side_b: u32, corner: u32) -> u32 {
    VERTEX_AO[(side_a | (side_b << 1) | (corner << 2)) as usize]
}

#[inline(always)]
fn block_occludes_ambient_light_bit(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    offset: isize,
) -> u32 {
    let index = (padded_index as isize + offset) as usize;
    debug_assert!(index < PADDED_CHUNK_VOLUME);
    unsafe {
        ((block_mesh_flags(*blocks.blocks.get_unchecked(index)) & BLOCK_FLAG_FULL_CUBE) >> 1) as u32
    }
}
