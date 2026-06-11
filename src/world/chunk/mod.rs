pub mod collider;
pub mod mesh;

use bevy::prelude::*;
use collider::ChunkColliderPlugin;
use mesh::ChunkMeshPlugin;

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
pub const CHUNK_BLOCK_STORAGE_BYTES: usize = CHUNK_VOLUME * std::mem::size_of::<u16>();

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkPosition(pub IVec3);

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsSave;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsMeshRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsColliderRebuild;

#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect)]
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

        if !block.is_solid() {
            return false;
        }

        if old_block.is_solid() {
            return false;
        };

        *old_block = block;

        true
    }

    pub fn break_block(&mut self, pos: UVec3) -> bool {
        let block = self.get_mut_uvec(pos);

        if !block.is_solid() {
            return false;
        };

        *block = BlockType::Air;

        true
    }

    pub fn iter(&self) -> BlockIterator<'_> {
        BlockIterator {
            chunk: self,
            x: 0,
            y: 0,
            z: 0,
        }
    }

    pub fn to_storage_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(CHUNK_BLOCK_STORAGE_BYTES);

        for (block, _) in self.iter() {
            bytes.extend_from_slice(&block.storage_id().to_le_bytes());
        }

        bytes
    }

    pub fn try_from_storage_bytes(bytes: &[u8]) -> Result<Self, ChunkDecodeError> {
        if bytes.len() != CHUNK_BLOCK_STORAGE_BYTES {
            return Err(ChunkDecodeError::InvalidLength {
                expected: CHUNK_BLOCK_STORAGE_BYTES,
                actual: bytes.len(),
            });
        }

        let mut chunk = Chunk::default();
        let mut offset = 0;

        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let id = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
                    chunk.blocks[x][z][y] = BlockType::from_storage_id(id)
                        .ok_or(ChunkDecodeError::UnknownBlockId(id))?;
                    offset += std::mem::size_of::<u16>();
                }
            }
        }

        Ok(chunk)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkDecodeError {
    InvalidLength { expected: usize, actual: usize },
    UnknownBlockId(u16),
}

impl std::fmt::Display for ChunkDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    f,
                    "invalid chunk byte length: expected {expected}, got {actual}"
                )
            }
            Self::UnknownBlockId(id) => write!(f, "unknown block storage id {id}"),
        }
    }
}

impl std::error::Error for ChunkDecodeError {}

pub struct BlockIterator<'a> {
    chunk: &'a Chunk,
    x: usize,
    y: usize,
    z: usize,
}

impl<'a> Iterator for BlockIterator<'a> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_storage_bytes_roundtrip_in_iteration_order() {
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Grass;
        chunk.blocks[1][0][0] = BlockType::Dirt;
        chunk.blocks[0][1][0] = BlockType::Stone;
        chunk.blocks[0][0][1] = BlockType::OakLog;
        chunk.blocks[15][15][15] = BlockType::OakLeaves;

        let bytes = chunk.to_storage_bytes();

        assert_eq!(bytes.len(), CHUNK_BLOCK_STORAGE_BYTES);
        assert_eq!(Chunk::try_from_storage_bytes(&bytes), Ok(chunk));
    }

    #[test]
    fn chunk_storage_bytes_reject_unknown_block_ids() {
        let mut bytes = Chunk::default().to_storage_bytes();
        bytes[0..2].copy_from_slice(&u16::MAX.to_le_bytes());

        assert_eq!(
            Chunk::try_from_storage_bytes(&bytes),
            Err(ChunkDecodeError::UnknownBlockId(u16::MAX))
        );
    }
}
