use bevy::{platform::collections::HashMap, prelude::*};
use serde::{Deserialize, Serialize};

use super::super::{
    CHUNK_ISIZE, CHUNK_SIZE, LocalBlockPos, chunk_neighbor_offsets,
    neighborhood::{NeighborOffset, PADDED_CHUNK_VOLUME, PaddedChunkIndex, padded_chunk_index},
};

pub(super) const SKY_LIGHT_MAX: u8 = 15;

pub(super) const PADDED_LIGHT_WORDS: usize = PADDED_CHUNK_VOLUME.div_ceil(4);
const MISSING_PADDED_LIGHT_WORD: u32 = 0xF0F0F0F0;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ChunkLight {
    light: [[[u8; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
}

impl Default for ChunkLight {
    fn default() -> Self {
        Self {
            light: [[[0u8; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
        }
    }
}

impl ChunkLight {
    pub fn sky_light(&self, pos: UVec3) -> u8 {
        let packed = self.light[pos.x as usize][pos.z as usize][pos.y as usize];
        packed >> 4
    }

    pub fn block_light(&self, pos: UVec3) -> u8 {
        let packed = self.light[pos.x as usize][pos.z as usize][pos.y as usize];
        packed & 0x0F
    }

    pub fn packed_light(&self, pos: UVec3) -> u8 {
        self.light[pos.x as usize][pos.z as usize][pos.y as usize]
    }

    pub fn packed_light_at(&self, x: usize, z: usize, y: usize) -> u8 {
        self.light[x][z][y]
    }

    /// Build the padded light buffer consumed by chunk mesh rendering.
    ///
    /// `center_pos` is the chunk's position in chunk coords,
    /// `lights` is a map of all available chunks' light data (keyed by chunk position).
    /// Returns a flat array of u32 values, with four packed padded cells per word.
    /// Cell layout before packing: `index = x + z * 18 + y * 18 * 18`.
    pub fn build_padded_light_data(
        center_pos: IVec3,
        lights: &HashMap<IVec3, &ChunkLight>,
    ) -> Box<[u32]> {
        let mut data = vec![MISSING_PADDED_LIGHT_WORD; PADDED_LIGHT_WORDS].into_boxed_slice();

        // Copy center chunk (offset 0,0,0)
        if let Some(center) = lights.get(&center_pos) {
            for x in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for y in 0..CHUNK_SIZE {
                        let padded_idx = PaddedChunkIndex::from_local(LocalBlockPos::new(
                            x as u32, y as u32, z as u32,
                        ))
                        .as_usize();
                        write_padded_light(&mut data, padded_idx, center.packed_light_at(x, z, y));
                    }
                }
            }
        }

        // Copy neighbor chunks' border regions
        for offset in chunk_neighbor_offsets() {
            let Some(neighbor) = lights.get(&(center_pos + offset)) else {
                continue;
            };

            for x in NeighborOffset::source_axis_range(offset.x) {
                for z in NeighborOffset::source_axis_range(offset.z) {
                    for y in NeighborOffset::source_axis_range(offset.y) {
                        let px = (x as i32 + offset.x * CHUNK_ISIZE) + 1;
                        let pz = (z as i32 + offset.z * CHUNK_ISIZE) + 1;
                        let py = (y as i32 + offset.y * CHUNK_ISIZE) + 1;
                        let padded_idx = padded_chunk_index(px as usize, py as usize, pz as usize);
                        write_padded_light(
                            &mut data,
                            padded_idx,
                            neighbor.packed_light_at(x, z, y),
                        );
                    }
                }
            }
        }

        data
    }

    pub fn set_sky_light(&mut self, pos: UVec3, value: u8) {
        let slot = &mut self.light[pos.x as usize][pos.z as usize][pos.y as usize];
        *slot = (*slot & 0x0F) | ((value & 0x0F) << 4);
    }

    pub fn set_block_light(&mut self, pos: UVec3, value: u8) {
        let slot = &mut self.light[pos.x as usize][pos.z as usize][pos.y as usize];
        *slot = (*slot & 0xF0) | (value & 0x0F);
    }

    pub fn reset_all_sky_light(&mut self) {
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    self.light[x][z][y] &= 0x0F;
                }
            }
        }
    }

    pub fn reset_all_block_light(&mut self) {
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    self.light[x][z][y] &= 0xF0;
                }
            }
        }
    }
}

fn write_padded_light(data: &mut [u32], padded_idx: usize, packed_light: u8) {
    let word_idx = padded_idx / 4;
    let shift = (padded_idx % 4) * 8;
    let mask = 0xFFu32 << shift;
    data[word_idx] = (data[word_idx] & !mask) | ((packed_light as u32) << shift);
}

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkHeightmap {
    pub heights: [[u8; CHUNK_SIZE]; CHUNK_SIZE],
}

impl ChunkHeightmap {
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, bincode::config::standard())
            .expect("ChunkHeightmap serialization is infallible")
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        bincode::serde::decode_from_slice(bytes, bincode::config::standard())
            .map(|(hm, _)| hm)
            .unwrap_or_default()
    }
}
