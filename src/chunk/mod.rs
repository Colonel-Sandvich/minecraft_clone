pub mod collider;
pub mod mesh;
pub mod util;

use bevy::{math::uvec3, prelude::*};
use collider::ChunkColliderPlugin;
use mesh::ChunkMeshPlugin;
use rand::Rng;
use strum::EnumCount;

use crate::block::BlockType;

pub struct ChunkPlugin;

impl Plugin for ChunkPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ChunkMeshPlugin);
        app.add_plugins(ChunkColliderPlugin);
    }
}

pub const CHUNK_SIZE: usize = 16;
pub const CHUNK_ISIZE: i32 = 16;

pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

#[derive(Component, Debug, Clone, Reflect)]
pub struct Chunk {
    pub blocks: [[[BlockType; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
}

impl Default for Chunk {
    fn default() -> Self {
        Self {
            blocks: [[[BlockType::Air; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
        }
    }
}

#[derive(Bundle, Default)]
pub struct ChunkBundle {
    pub chunk: Chunk,
    pub spatial: SpatialBundle,
}

impl Chunk {
    pub fn get(&self, pos: UVec3) -> BlockType {
        self.blocks[pos.x as usize][pos.z as usize][pos.y as usize]
    }

    pub fn get_i(&self, x: i32, y: i32, z: i32) -> Option<BlockType> {
        let outside = |a: i32| !(0..CHUNK_ISIZE).contains(&a);
        if outside(x) || outside(y) || outside(z) {
            return None;
        }

        Some(self.blocks[x as usize][z as usize][y as usize])
    }

    pub fn get_mut(&mut self, x: u32, y: u32, z: u32) -> Option<&mut BlockType> {
        let outside = |a: u32| !(0..CHUNK_SIZE).contains(&(a as usize));
        if outside(x) || outside(y) || outside(z) {
            return None;
        }

        Some(&mut self.blocks[x as usize][z as usize][y as usize])
    }

    pub fn get_mut_uvec(&mut self, pos: UVec3) -> &mut BlockType {
        self.get_mut(pos.x, pos.y, pos.z).unwrap()
    }

    pub fn place_block(&mut self, pos: UVec3, block: BlockType) -> bool {
        let old_block = self.get_mut_uvec(pos);

        if old_block.is_solid() {
            return false;
        };

        *old_block = block;

        true
    }

    pub fn place_random_block(&mut self) -> Option<(BlockType, UVec3)> {
        let mut rng = rand::thread_rng();
        let mut get_range = || rng.gen_range(0..CHUNK_SIZE);

        let pos = uvec3(get_range() as u32, get_range() as u32, get_range() as u32);
        let block = self.get_mut_uvec(pos);

        if !block.is_solid() {
            // Assumes Air = 0
            *block = BlockType::from_repr(rng.gen_range(1..BlockType::COUNT)).unwrap();

            return Some((block.clone(), pos));
        }

        None
    }

    pub fn break_block(&mut self, pos: UVec3) -> bool {
        let block = self.get_mut_uvec(pos);

        if !block.is_solid() {
            return false;
        };

        *block = BlockType::Air;

        true
    }

    pub fn iter(&self) -> ChunkIterator {
        ChunkIterator {
            chunk: self,
            x: 0,
            y: 0,
            z: 0,
        }
    }
}

pub struct ChunkIterator<'a> {
    chunk: &'a Chunk,
    x: usize,
    y: usize,
    z: usize,
}

impl<'a> Iterator for ChunkIterator<'a> {
    type Item = (&'a BlockType, (usize, usize, usize));

    fn next(&mut self) -> Option<Self::Item> {
        if self.y >= CHUNK_SIZE {
            return None;
        }

        let pos = (self.x, self.y, self.z);
        let block = &self.chunk.blocks[self.x][self.z][self.y];

        self.x += 1;
        if self.x >= CHUNK_SIZE {
            self.x = 0;
            self.z += 1;
            if self.z >= CHUNK_SIZE {
                self.z = 0;
                self.y += 1;
            }
        }

        Some((block, pos))
    }
}
