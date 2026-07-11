use bevy::platform::collections::HashMap;

use super::super::{
    CHUNK_ISIZE, CHUNK_SIZE, ChunkLight, ChunkPos, LocalBlockPos, chunk_neighbor_offsets,
    neighborhood::{NeighborOffset, PADDED_CHUNK_VOLUME, PaddedChunkIndex, padded_chunk_index},
};
use super::ChunkMeshLight;

const PADDED_LIGHT_WORDS: usize = PADDED_CHUNK_VOLUME.div_ceil(4);
const MISSING_PADDED_LIGHT_WORD: u32 = 0xF0F0F0F0;

impl ChunkMeshLight {
    /// Builds the one-cell light halo consumed by chunk mesh rendering.
    ///
    /// Four packed light cells are stored in each output word. Missing chunks
    /// use full sky light and zero block light, matching an open boundary.
    pub fn build_padded_data(
        center: ChunkPos,
        lights: &HashMap<ChunkPos, &ChunkLight>,
    ) -> Box<[u32]> {
        let mut data = vec![MISSING_PADDED_LIGHT_WORD; PADDED_LIGHT_WORDS].into_boxed_slice();

        if let Some(center_light) = lights.get(&center) {
            for x in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for y in 0..CHUNK_SIZE {
                        let local = LocalBlockPos::new(x as u32, y as u32, z as u32);
                        let padded = PaddedChunkIndex::from_local(local).as_usize();
                        write_padded_light(&mut data, padded, center_light.packed_light(local));
                    }
                }
            }
        }

        for offset in chunk_neighbor_offsets() {
            let Some(neighbor) = lights.get(&center.offset(offset)) else {
                continue;
            };

            for x in NeighborOffset::source_axis_range(offset.x) {
                for z in NeighborOffset::source_axis_range(offset.z) {
                    for y in NeighborOffset::source_axis_range(offset.y) {
                        let local = LocalBlockPos::new(x as u32, y as u32, z as u32);
                        let px = x as i32 + offset.x * CHUNK_ISIZE + 1;
                        let pz = z as i32 + offset.z * CHUNK_ISIZE + 1;
                        let py = y as i32 + offset.y * CHUNK_ISIZE + 1;
                        let padded = padded_chunk_index(px as usize, py as usize, pz as usize);
                        write_padded_light(&mut data, padded, neighbor.packed_light(local));
                    }
                }
            }
        }

        data
    }
}

fn write_padded_light(data: &mut [u32], padded: usize, light: u8) {
    let word = padded / 4;
    let shift = (padded % 4) * 8;
    let mask = 0xFFu32 << shift;
    data[word] = (data[word] & !mask) | (u32::from(light) << shift);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unpack(data: &[u32], index: usize) -> u8 {
        ((data[index / 4] >> ((index % 4) * 8)) & 0xFF) as u8
    }

    #[test]
    fn padded_data_maps_center_face_edge_and_corner_chunks() {
        let center_pos = ChunkPos::new(-3, 2, 7);
        let center_local = LocalBlockPos::ZERO;
        let mut center = ChunkLight::default();
        center.set_sky_light(center_local, 1);
        center.set_block_light(center_local, 2);

        let right_local = LocalBlockPos::ZERO;
        let mut right = ChunkLight::default();
        right.set_sky_light(right_local, 10);
        right.set_block_light(right_local, 11);

        let edge_local = LocalBlockPos::new(0, 0, 7);
        let mut upper_right = ChunkLight::default();
        upper_right.set_sky_light(edge_local, 3);
        upper_right.set_block_light(edge_local, 4);

        let corner_local = LocalBlockPos::MAX;
        let mut lower_left_forward = ChunkLight::default();
        lower_left_forward.set_sky_light(corner_local, 5);
        lower_left_forward.set_block_light(corner_local, 6);

        let lights = HashMap::from([
            (center_pos, &center),
            (center_pos.offset(bevy::math::IVec3::X), &right),
            (
                center_pos.offset(bevy::math::IVec3::X + bevy::math::IVec3::Y),
                &upper_right,
            ),
            (
                center_pos.offset(bevy::math::IVec3::NEG_ONE),
                &lower_left_forward,
            ),
        ]);
        let data = ChunkMeshLight::build_padded_data(center_pos, &lights);

        assert_eq!(data.len(), PADDED_LIGHT_WORDS);
        assert_eq!(unpack(&data, padded_chunk_index(1, 1, 1)), 0x12);
        assert_eq!(unpack(&data, padded_chunk_index(17, 1, 1)), 0xAB);
        assert_eq!(unpack(&data, padded_chunk_index(17, 17, 8)), 0x34);
        assert_eq!(unpack(&data, padded_chunk_index(0, 0, 0)), 0x56);
        assert_eq!(unpack(&data, padded_chunk_index(0, 17, 17)), 0xF0);
    }
}
