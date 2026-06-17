pub mod ambient_occlusion;
pub mod collider;
pub mod light;
pub mod mesh;

use ambient_occlusion::AmbientOcclusionPlugin;
use bevy::prelude::*;
use collider::ChunkColliderPlugin;
use mesh::ChunkMeshPlugin;

pub use light::{
    ChunkHeightmap, ChunkLight, compute_light, light_on_place_block, light_on_place_sky,
};

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

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsLightRebuild;

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
        let outside = |a: i32| a < 0 || a >= CHUNK_ISIZE;
        if outside(x) || outside(y) || outside(z) {
            return None;
        }

        Some(self.blocks[x as usize][z as usize][y as usize])
    }

    pub(crate) fn get_mut(&mut self, x: u32, y: u32, z: u32) -> Option<&mut BlockType> {
        let outside = |a: u32| a >= CHUNK_SIZE as u32;
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

    fn build_palette(&self) -> Vec<BlockType> {
        let mut palette = Vec::new();
        for (block, _) in self.iter() {
            if !palette.contains(block) {
                palette.push(*block);
            }
        }
        palette
    }

    /// Bit-packed palette encoding.
    ///
    /// ```text
    /// [u16 LE: palette_size]
    /// for each entry: [u8: name_len] [name bytes]
    /// [u8: bits_per_index]
    /// [bit-packed body, MSB-first, padded to byte boundary]
    /// [4096 bytes: light data (format v2 only, removed in v3)]
    /// ```
    /// bits_per_index = ceil(log2(palette_size)), min 1.
    pub fn to_storage_bytes(&self) -> Vec<u8> {
        let palette = self.build_palette();
        let bits = bits_for(palette.len());
        let block_to_idx: std::collections::HashMap<BlockType, u8> = palette
            .iter()
            .enumerate()
            .map(|(i, &b)| (b, i as u8))
            .collect();
        let indices: Vec<u8> = self.iter().map(|(b, _)| block_to_idx[&b]).collect();

        let mut bytes = Vec::new();

        // header
        bytes.extend_from_slice(&(palette.len() as u16).to_le_bytes());
        for &b in &palette {
            let name = b.to_string();
            bytes.push(name.len() as u8);
            bytes.extend_from_slice(name.as_bytes());
        }
        bytes.push(bits);

        // bit-packed body
        let body_start = bytes.len();
        let body_bytes = (indices.len() * bits as usize + 7) / 8;
        bytes.resize(body_start + body_bytes, 0);
        pack(&mut bytes[body_start..], &indices, bits);

        bytes
    }

    pub fn try_from_storage_bytes(
        bytes: &[u8],
        format_version: u32,
    ) -> Result<(Self, ChunkLight), ChunkDecodeError> {
        if bytes.len() < 3 {
            return Err(ChunkDecodeError::Truncated);
        }

        let palette_size = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        if palette_size == 0 {
            return Err(ChunkDecodeError::Truncated);
        }

        let mut pos = 2usize;
        let mut palette = Vec::with_capacity(palette_size);

        for _ in 0..palette_size {
            if pos >= bytes.len() {
                return Err(ChunkDecodeError::Truncated);
            }
            let len = bytes[pos] as usize;
            pos += 1;
            if pos + len > bytes.len() {
                return Err(ChunkDecodeError::Truncated);
            }
            let name = std::str::from_utf8(&bytes[pos..pos + len])
                .map_err(|_| ChunkDecodeError::InvalidHeader)?;
            pos += len;
            let block = BlockType::from_name(name)
                .ok_or_else(|| ChunkDecodeError::UnknownBlock(name.to_owned()))?;
            palette.push(block);
        }

        let bits = *bytes.get(pos).ok_or(ChunkDecodeError::Truncated)?;
        pos += 1;

        let body = bytes.get(pos..).ok_or(ChunkDecodeError::Truncated)?;
        let mask = (1u8 << bits) - 1;

        let mut chunk = Chunk::default();
        let mut bit_pos = 0usize;

        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let idx =
                        read_bits(body, &mut bit_pos, bits).ok_or(ChunkDecodeError::Truncated)?;
                    let idx = (idx & mask) as usize;
                    if idx >= palette.len() {
                        return Err(ChunkDecodeError::InvalidHeader);
                    }
                    chunk.blocks[x][z][y] = palette[idx];
                }
            }
        }

        let block_body_bits = bit_pos;
        let block_body_bytes = (block_body_bits + 7) / 8;
        let light_start = pos + block_body_bytes;

        let mut light = ChunkLight::default();
        if format_version == 2 {
            if light_start + CHUNK_VOLUME > bytes.len() {
                return Err(ChunkDecodeError::Truncated);
            }
            let light_bytes = &bytes[light_start..light_start + CHUNK_VOLUME];
            let mut idx = 0;
            for x in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for y in 0..CHUNK_SIZE {
                        light.light[x][z][y] = light_bytes[idx];
                        idx += 1;
                    }
                }
            }
        }

        Ok((chunk, light))
    }
}

fn bits_for(palette_size: usize) -> u8 {
    match palette_size {
        0 | 1 => 1,
        n => (usize::BITS - (n - 1).leading_zeros()) as u8,
    }
}

/// Pack `indices` (each `bits` wide, MSB-first) into `buf`.
#[inline]
fn pack(buf: &mut [u8], indices: &[u8], bits: u8) {
    let mut bp = 0usize;
    for &idx in indices {
        let mut val = idx;
        for _ in 0..bits {
            buf[bp >> 3] |= ((val >> (bits - 1)) & 1) << (7 - (bp & 7));
            val <<= 1;
            bp += 1;
        }
    }
}

/// Read one `bits`-wide value from `buf` at the current `bit_pos`.
#[inline]
fn read_bits(buf: &[u8], bit_pos: &mut usize, bits: u8) -> Option<u8> {
    let mut val = 0u8;
    for _ in 0..bits {
        let byte = buf.get(*bit_pos >> 3)?;
        let bit = (byte >> (7 - (*bit_pos & 7))) & 1;
        val = (val << 1) | bit;
        *bit_pos += 1;
    }
    Some(val)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkDecodeError {
    Truncated,
    InvalidHeader,
    UnknownBlock(String),
}

impl std::fmt::Display for ChunkDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "chunk data truncated"),
            Self::InvalidHeader => write!(f, "invalid chunk header"),
            Self::UnknownBlock(name) => write!(f, "unknown block: {name}"),
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
        let (decoded, decoded_light) = Chunk::try_from_storage_bytes(&bytes, 3).unwrap();
        assert_eq!(decoded, chunk);
        assert_eq!(decoded_light, ChunkLight::default());
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_all_air() {
        let chunk = Chunk::default();
        let bytes = chunk.to_storage_bytes();
        let (decoded, decoded_light) = Chunk::try_from_storage_bytes(&bytes, 3).unwrap();
        assert_eq!(decoded, chunk);
        assert_eq!(decoded_light, ChunkLight::default());
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_full_stone() {
        let mut chunk = Chunk::default();
        chunk.blocks = [[[BlockType::Stone; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];
        let bytes = chunk.to_storage_bytes();
        let (decoded, decoded_light) = Chunk::try_from_storage_bytes(&bytes, 3).unwrap();
        assert_eq!(decoded, chunk);
        assert_eq!(decoded_light, ChunkLight::default());
    }

    #[test]
    fn v3_format_never_stores_light() {
        let chunk = Chunk::default();
        let mut light = ChunkLight::default();
        light.set_sky_light(uvec3(8, 8, 8), 15);
        light.set_block_light(uvec3(8, 8, 8), 7);

        let bytes = chunk.to_storage_bytes();
        let (_, decoded_light) = Chunk::try_from_storage_bytes(&bytes, 3).unwrap();
        assert_eq!(decoded_light.packed_light(uvec3(8, 8, 8)), 0);
    }

    #[test]
    fn v2_format_still_reads_stored_light() {
        let chunk = Chunk::default();
        let mut light = ChunkLight::default();
        light.set_sky_light(uvec3(8, 8, 8), 15);
        light.set_block_light(uvec3(8, 8, 8), 7);

        let mut bytes = chunk.to_storage_bytes();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    bytes.push(light.light[x][z][y]);
                }
            }
        }
        let (_, decoded_light) = Chunk::try_from_storage_bytes(&bytes, 2).unwrap();
        assert_eq!(decoded_light.sky_light(uvec3(8, 8, 8)), 15);
        assert_eq!(decoded_light.block_light(uvec3(8, 8, 8)), 7);
    }

    #[test]
    fn v1_format_loads_v3_blob_with_default_light() {
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Grass;
        let bytes = chunk.to_storage_bytes();
        let (decoded, decoded_light) = Chunk::try_from_storage_bytes(&bytes, 1).unwrap();
        assert_eq!(decoded.blocks[0][0][0], BlockType::Grass);
        assert_eq!(decoded_light.packed_light(uvec3(0, 0, 0)), 0);
        assert_eq!(decoded_light.packed_light(uvec3(8, 8, 8)), 0);
    }

    #[test]
    fn chunk_storage_bytes_reject_garbled_data() {
        assert!(Chunk::try_from_storage_bytes(&[], 2).is_err());
    }

    #[test]
    fn chunk_storage_bytes_reject_unknown_block_name() {
        let name = b"nonexistent";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1u16.to_le_bytes()); // palette_size = 1
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name);
        bytes.push(1); // bits_per_index
        bytes.resize(bytes.len() + 512, 0);

        match Chunk::try_from_storage_bytes(&bytes, 2) {
            Err(ChunkDecodeError::UnknownBlock(n)) => assert_eq!(n, "nonexistent"),
            other => panic!("expected UnknownBlock, got {other:?}"),
        }
    }
}
