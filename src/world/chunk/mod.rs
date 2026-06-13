pub mod ambient_occlusion;
pub mod collider;
pub mod mesh;

use ambient_occlusion::AmbientOcclusionPlugin;
use bevy::prelude::*;
use collider::ChunkColliderPlugin;
use mesh::ChunkMeshPlugin;

use crate::block::BlockType;

pub struct ChunkPlugin;

impl Plugin for ChunkPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(AmbientOcclusionPlugin);
        app.add_plugins(ChunkMeshPlugin);
        app.add_plugins(ChunkColliderPlugin);
    }
}

pub const CHUNK_SIZE: usize = 16;
pub const CHUNK_ISIZE: i32 = 16;

pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;
pub const CHUNK_BLOCK_STORAGE_BYTES: usize = CHUNK_VOLUME * std::mem::size_of::<u16>();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockDelta {
    pub old: BlockType,
    pub new: BlockType,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChunkBlockCounts {
    pub rendered: u16,
    pub full_cubes: u16,
    pub translucent: u16,
}

impl ChunkBlockCounts {
    pub fn apply_delta(&mut self, delta: BlockDelta) {
        let (old_rendered, old_full, old_trans) = block_counts(delta.old);
        let (new_rendered, new_full, new_trans) = block_counts(delta.new);
        self.rendered = self
            .rendered
            .wrapping_add(new_rendered)
            .wrapping_sub(old_rendered);
        self.full_cubes = self
            .full_cubes
            .wrapping_add(new_full)
            .wrapping_sub(old_full);
        self.translucent = self
            .translucent
            .wrapping_add(new_trans)
            .wrapping_sub(old_trans);
    }
}

fn block_counts(block: BlockType) -> (u16, u16, u16) {
    let rendered = block.is_rendered() as u16;
    let full_cubes = block.is_full_cube() as u16;
    (rendered, full_cubes, rendered.saturating_sub(full_cubes))
}

pub(crate) fn chunk_neighbor_offsets() -> impl Iterator<Item = IVec3> {
    (-1..=1).flat_map(|x| {
        (-1..=1).flat_map(move |y| {
            (-1..=1).filter_map(move |z| {
                let offset = ivec3(x, y, z);
                (offset != IVec3::ZERO).then_some(offset)
            })
        })
    })
}

pub(crate) fn chunk_neighbor_offsets_for_block(block: UVec3) -> impl Iterator<Item = IVec3> {
    chunk_neighbor_offsets().filter(move |offset| {
        neighbor_axis_can_sample_block(offset.x, block.x)
            && neighbor_axis_can_sample_block(offset.y, block.y)
            && neighbor_axis_can_sample_block(offset.z, block.z)
    })
}

fn neighbor_axis_can_sample_block(offset: i32, coord: u32) -> bool {
    match offset {
        -1 => coord == 0,
        0 => true,
        1 => coord == CHUNK_SIZE as u32 - 1,
        _ => false,
    }
}

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
    pub fn get_block(&self, pos: UVec3) -> BlockType {
        self.blocks[pos.x as usize][pos.z as usize][pos.y as usize]
    }

    pub fn set_block(&mut self, pos: UVec3, block: BlockType) -> BlockDelta {
        let old = self.blocks[pos.x as usize][pos.z as usize][pos.y as usize];
        self.blocks[pos.x as usize][pos.z as usize][pos.y as usize] = block;
        BlockDelta { old, new: block }
    }

    pub fn get_i(&self, x: i32, y: i32, z: i32) -> Option<BlockType> {
        let outside = |a: i32| !(0..CHUNK_ISIZE).contains(&a);
        if outside(x) || outside(y) || outside(z) {
            return None;
        }

        Some(self.blocks[x as usize][z as usize][y as usize])
    }

    pub(crate) fn get_mut(&mut self, x: u32, y: u32, z: u32) -> Option<&mut BlockType> {
        let outside = |a: u32| !(0..CHUNK_SIZE).contains(&(a as usize));
        if outside(x) || outside(y) || outside(z) {
            return None;
        }

        Some(&mut self.blocks[x as usize][z as usize][y as usize])
    }

    pub(crate) fn get_mut_uvec(&mut self, pos: UVec3) -> &mut BlockType {
        self.get_mut(pos.x, pos.y, pos.z).unwrap()
    }

    pub fn place_block(&mut self, pos: UVec3, block: BlockType) -> Option<BlockDelta> {
        if !block.is_solid() {
            return None;
        }

        let old_block = self.get_mut_uvec(pos);

        if old_block.is_solid() {
            return None;
        };

        let old = *old_block;
        *old_block = block;

        Some(BlockDelta { old, new: block })
    }

    pub fn break_block(&mut self, pos: UVec3) -> Option<BlockDelta> {
        let block = self.get_mut_uvec(pos);

        if !block.is_solid() {
            return None;
        };

        let old = *block;
        *block = BlockType::Air;

        Some(BlockDelta {
            old,
            new: BlockType::Air,
        })
    }

    pub fn compute_block_counts(&self) -> ChunkBlockCounts {
        let mut counts = ChunkBlockCounts::default();
        for (block, _) in self.iter() {
            let (r, fc, t) = block_counts(*block);
            counts.rendered += r;
            counts.full_cubes += fc;
            counts.translucent += t;
        }
        counts
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
    fn chunk_neighbor_offsets_cover_all_adjacent_chunks() {
        let offsets = chunk_neighbor_offsets().collect::<Vec<_>>();

        assert_eq!(offsets.len(), 26);
        assert!(!offsets.contains(&IVec3::ZERO));
        assert!(offsets.contains(&IVec3::NEG_X));
        assert!(offsets.contains(&ivec3(1, 1, 1)));
    }

    #[test]
    fn block_boundary_neighbor_offsets_cover_faces_edges_and_corners() {
        assert_eq!(chunk_neighbor_offsets_for_block(uvec3(1, 2, 3)).count(), 0);
        assert_eq!(
            chunk_neighbor_offsets_for_block(uvec3(0, 2, 3)).collect::<Vec<_>>(),
            vec![IVec3::NEG_X]
        );

        let edge_offsets = chunk_neighbor_offsets_for_block(uvec3(0, 0, 3)).collect::<Vec<_>>();
        assert_eq!(edge_offsets.len(), 3);
        assert!(edge_offsets.contains(&IVec3::NEG_X));
        assert!(edge_offsets.contains(&IVec3::NEG_Y));
        assert!(edge_offsets.contains(&ivec3(-1, -1, 0)));

        assert_eq!(chunk_neighbor_offsets_for_block(UVec3::ZERO).count(), 7);
    }

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
