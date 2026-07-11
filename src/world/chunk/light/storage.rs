use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::super::{CHUNK_SIZE, LocalBlockPos};

pub(super) const SKY_LIGHT_MAX: u8 = 15;

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
    pub fn sky_light(&self, pos: LocalBlockPos) -> u8 {
        let packed = self.light[pos.x()][pos.z()][pos.y()];
        packed >> 4
    }

    pub fn block_light(&self, pos: LocalBlockPos) -> u8 {
        let packed = self.light[pos.x()][pos.z()][pos.y()];
        packed & 0x0F
    }

    pub fn packed_light(&self, pos: LocalBlockPos) -> u8 {
        self.light[pos.x()][pos.z()][pos.y()]
    }

    pub fn set_sky_light(&mut self, pos: LocalBlockPos, value: u8) {
        let slot = &mut self.light[pos.x()][pos.z()][pos.y()];
        *slot = (*slot & 0x0F) | ((value & 0x0F) << 4);
    }

    pub fn set_block_light(&mut self, pos: LocalBlockPos, value: u8) {
        let slot = &mut self.light[pos.x()][pos.z()][pos.y()];
        *slot = (*slot & 0xF0) | (value & 0x0F);
    }
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
