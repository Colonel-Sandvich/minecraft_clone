//! Water surface shaping and packed per-face water metadata.

use crate::block::WATER_RENDER_ID;

use super::super::{
    blocks::{ChunkMeshBlocks, DIRECTION_COUNT, DIRECTION_INDEX_OFFSETS, PADDED_CHUNK_SIZE},
    face::PackedFace,
};

pub(super) struct WaterFaceData {
    packed_by_side: [u32; DIRECTION_COUNT],
    info: u32,
}

impl WaterFaceData {
    pub(super) fn from_cell(blocks: &ChunkMeshBlocks, padded_index: usize) -> Self {
        let level = blocks.get_fluid_level(padded_index);
        let (h00, h10, h01, h11) = water_corner_heights(level, blocks, padded_index);
        let corner_face = PackedFace::default().with_corner_heights(h00, h10, h01, h11);
        let [corner_packed, corner_info] = corner_face.words();
        let mut packed_by_side = [corner_packed; DIRECTION_COUNT];

        let below_index = (padded_index as isize + DIRECTION_INDEX_OFFSETS[2]) as usize;
        let below = unsafe { *blocks.blocks.get_unchecked(below_index) };
        if below == WATER_RENDER_ID {
            let below_level = blocks.get_fluid_level(below_index);
            let (bh00, bh10, bh01, bh11) = water_corner_heights(below_level, blocks, below_index);
            for (side_index, packed) in packed_by_side.iter_mut().enumerate() {
                let (lo, hi) = water_below_pair(side_index, bh00, bh10, bh01, bh11);
                *packed |= PackedFace::default().with_water_below(lo, hi).words()[0];
            }
        }

        let flow_code = water_flow_code(level, blocks, padded_index);
        if flow_code != 0 || (h00 | h10 | h01 | h11) != 8 {
            packed_by_side[3] |= PackedFace::default().with_water_up_flow(flow_code).words()[0];
        }

        Self {
            packed_by_side,
            info: corner_info,
        }
    }

    #[inline(always)]
    pub(super) fn apply(&self, mut face: PackedFace, side_index: usize) -> PackedFace {
        face.apply_packed_bits(self.packed_by_side[side_index], self.info);
        face
    }
}

/// Compute the four top-surface corner heights in ninths of a block.
pub(crate) fn water_corner_heights(
    self_level: u8,
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
) -> (u32, u32, u32, u32) {
    let base = padded_index as isize;
    let padded_size = PADDED_CHUNK_SIZE as isize;
    let negative_x = base - 1;
    let positive_x = base + 1;
    let negative_z = base - padded_size;
    let positive_z = base + padded_size;
    let self_height = water_height_at(blocks, base).unwrap_or_else(|| self_level.min(8) as i32);
    if self_height >= 9 {
        return (9, 9, 9, 9);
    }

    (
        water_corner_height(
            self_height,
            blocks,
            negative_x,
            negative_z,
            negative_x - padded_size,
        ),
        water_corner_height(
            self_height,
            blocks,
            positive_x,
            negative_z,
            positive_x - padded_size,
        ),
        water_corner_height(
            self_height,
            blocks,
            negative_x,
            positive_z,
            negative_x + padded_size,
        ),
        water_corner_height(
            self_height,
            blocks,
            positive_x,
            positive_z,
            positive_x + padded_size,
        ),
    )
}

fn water_corner_height(
    self_height: i32,
    blocks: &ChunkMeshBlocks,
    adjacent_a: isize,
    adjacent_b: isize,
    diagonal: isize,
) -> u32 {
    let adjacent_a = water_height_at(blocks, adjacent_a);
    let adjacent_b = water_height_at(blocks, adjacent_b);

    if adjacent_a == Some(9) || adjacent_b == Some(9) {
        return 9;
    }

    let mut weighted = WeightedWaterHeight::default();
    if adjacent_a.is_some_and(|height| height > 0) || adjacent_b.is_some_and(|height| height > 0) {
        let diagonal = water_height_at(blocks, diagonal);
        if diagonal == Some(9) {
            return 9;
        }
        weighted.add(diagonal);
    }

    weighted.add(Some(self_height));
    weighted.add(adjacent_a);
    weighted.add(adjacent_b);
    weighted.average()
}

fn water_height_at(blocks: &ChunkMeshBlocks, padded_index: isize) -> Option<i32> {
    let index = padded_index as usize;
    let cell = unsafe { *blocks.blocks.get_unchecked(index) };
    if cell == WATER_RENDER_ID {
        let above_index = (padded_index + DIRECTION_INDEX_OFFSETS[3]) as usize;
        let above = unsafe { *blocks.blocks.get_unchecked(above_index) };
        if above == WATER_RENDER_ID {
            return Some(9);
        }
        return Some(blocks.get_fluid_level(index).min(8) as i32);
    }
    (cell == 0).then_some(0)
}

#[derive(Default)]
struct WeightedWaterHeight {
    total: i32,
    weight: i32,
}

impl WeightedWaterHeight {
    fn add(&mut self, height: Option<i32>) {
        let Some(height) = height else { return };
        let weight = if height >= 8 { 10 } else { 1 };
        self.total += height * weight;
        self.weight += weight;
    }

    fn average(self) -> u32 {
        if self.weight == 0 {
            return 0;
        }
        ((self.total + self.weight / 2) / self.weight).clamp(0, 9) as u32
    }
}

/// Lower-water corner pair for the bottom vertices of a side face.
#[inline(always)]
pub(crate) fn water_below_pair(
    side_index: usize,
    h00: u32,
    h10: u32,
    h01: u32,
    h11: u32,
) -> (u32, u32) {
    match side_index {
        0 => (h00, h01),
        1 => (h10, h11),
        4 => (h00, h10),
        5 => (h01, h11),
        _ => (0, 0),
    }
}

/// Quantized horizontal water-flow direction used for top-face UV rotation.
pub(crate) fn water_flow_code(
    self_level: u8,
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
) -> u32 {
    let mut dx = 0i32;
    let mut dz = 0i32;
    for (offset, vx, vz) in [
        (-1isize, -1, 0),
        (1, 1, 0),
        (-(PADDED_CHUNK_SIZE as isize), 0, -1),
        (PADDED_CHUNK_SIZE as isize, 0, 1),
    ] {
        let neighbor_index = (padded_index as isize + offset) as usize;
        let neighbor = unsafe { *blocks.blocks.get_unchecked(neighbor_index) };
        let neighbor_level = if neighbor == WATER_RENDER_ID {
            blocks.get_fluid_level(neighbor_index)
        } else if neighbor == 0 {
            0
        } else {
            continue;
        };

        let drop = self_level.saturating_sub(neighbor_level) as i32;
        if drop > 0 {
            dx += vx * drop;
            dz += vz * drop;
        }
    }

    quantized_water_flow_code(dx, dz)
}

fn quantized_water_flow_code(dx: i32, dz: i32) -> u32 {
    if dx == 0 && dz == 0 {
        return 0;
    }

    let absolute_x = dx.abs();
    let absolute_z = dz.abs();
    let sign_x = dx.signum();
    let sign_z = dz.signum();

    if absolute_z * 2 <= absolute_x {
        return if sign_x > 0 { 1 } else { 5 };
    }
    if absolute_x * 2 <= absolute_z {
        return if sign_z > 0 { 3 } else { 7 };
    }

    match (sign_x, sign_z) {
        (1, 1) => 2,
        (-1, 1) => 4,
        (-1, -1) => 6,
        (1, -1) => 8,
        _ => 0,
    }
}
