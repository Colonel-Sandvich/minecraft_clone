pub mod ambient_occlusion;
pub mod collider;
pub mod light;
pub mod mesh;

use bevy::prelude::*;
use collider::ChunkColliderPlugin;
use mesh::ChunkMeshPlugin;

pub use light::{ChunkHeightmap, ChunkLight};

use crate::block::BlockType;

pub struct ChunkPlugin;

impl Plugin for ChunkPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ChunkMeshPlugin);
        if std::env::var_os("MINECRAFT_CLONE_DISABLE_CHUNK_COLLIDERS").is_none() {
            app.add_plugins(ChunkColliderPlugin);
        }
    }
}

pub const CHUNK_SIZE: usize = 16;
pub const CHUNK_ISIZE: i32 = 16;

pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;
const FLUID_STORAGE_MAGIC: &[u8; 4] = b"FLD1";

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum FluidType {
    #[default]
    None,
    Water,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub struct FluidCell {
    pub ty: FluidType,
    pub level: u8,
}

impl FluidCell {
    pub const MAX_LEVEL: u8 = 8;

    pub const fn water_source() -> Self {
        Self {
            ty: FluidType::Water,
            level: Self::MAX_LEVEL,
        }
    }

    pub const fn is_empty(self) -> bool {
        matches!(self.ty, FluidType::None) || self.level == 0
    }

    fn storage_byte(self) -> u8 {
        match self.ty {
            FluidType::None => 0,
            FluidType::Water => (1 << 4) | self.level.min(Self::MAX_LEVEL),
        }
    }

    fn from_storage_byte(byte: u8) -> Result<Self, ChunkDecodeError> {
        let ty = match byte >> 4 {
            0 => FluidType::None,
            1 => FluidType::Water,
            _ => return Err(ChunkDecodeError::InvalidFluid),
        };
        let level = byte & 0x0f;
        if matches!(ty, FluidType::None) {
            return Ok(Self::default());
        }
        if level == 0 || level > Self::MAX_LEVEL {
            return Err(ChunkDecodeError::InvalidFluid);
        }
        Ok(Self { ty, level })
    }
}

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
pub struct ChunkNeedsLightUpload;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsColliderRebuild;

#[derive(Component, Debug, Clone, Copy)]
pub struct ChunkNeedsLightRebuild;

#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect)]
pub struct Chunk {
    pub blocks: [[[BlockType; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
    pub fluids: [[[FluidCell; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
}

impl Default for Chunk {
    fn default() -> Self {
        Self {
            blocks: [[[BlockType::Air; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
            fluids: [[[FluidCell::default(); CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
        }
    }
}

impl Chunk {
    pub fn get_block(&self, pos: UVec3) -> BlockType {
        self.blocks[pos.x as usize][pos.z as usize][pos.y as usize]
    }

    pub fn get_fluid(&self, pos: UVec3) -> FluidCell {
        self.fluids[pos.x as usize][pos.z as usize][pos.y as usize]
    }

    pub fn set_block(&mut self, pos: UVec3, block: BlockType) -> BlockDelta {
        let old = self.blocks[pos.x as usize][pos.z as usize][pos.y as usize];
        self.blocks[pos.x as usize][pos.z as usize][pos.y as usize] = block;
        self.fluids[pos.x as usize][pos.z as usize][pos.y as usize] = fluid_cell_for_block(block);
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
        if !block.is_placeable() {
            return None;
        }

        let old_block = self.get_mut_uvec(pos);

        if !old_block.can_be_replaced_by_placement() {
            return None;
        };

        let old = *old_block;
        *old_block = block;
        self.fluids[pos.x as usize][pos.z as usize][pos.y as usize] = fluid_cell_for_block(block);

        Some(BlockDelta { old, new: block })
    }

    pub fn break_block(&mut self, pos: UVec3) -> Option<BlockDelta> {
        let block = self.get_mut_uvec(pos);

        if !block.is_solid() {
            return None;
        };

        let old = *block;
        *block = BlockType::Air;
        self.fluids[pos.x as usize][pos.z as usize][pos.y as usize] = FluidCell::default();

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
        let indices: Vec<u8> = self.iter().map(|(b, _)| block_to_idx[b]).collect();

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
        let body_bytes = (indices.len() * bits as usize).div_ceil(8);
        bytes.resize(body_start + body_bytes, 0);
        pack(&mut bytes[body_start..], &indices, bits);

        if self.has_stored_fluids() {
            bytes.extend_from_slice(FLUID_STORAGE_MAGIC);
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        let fluid = self.fluids[x][z][y];
                        let block = self.blocks[x][z][y];
                        bytes.push(fluid.or_block_default(block).storage_byte());
                    }
                }
            }
        }

        bytes
    }

    pub fn try_from_storage_bytes(bytes: &[u8]) -> Result<Self, ChunkDecodeError> {
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
        let body_bytes = (CHUNK_VOLUME * bits as usize).div_ceil(8);

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

        pos += body_bytes;
        if let Some(fluid_bytes) = bytes.get(pos..) {
            if fluid_bytes.is_empty() {
                chunk.seed_water_fluids_from_blocks();
            } else {
                chunk.decode_fluid_storage(fluid_bytes)?;
            }
        }

        Ok(chunk)
    }

    fn has_stored_fluids(&self) -> bool {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    if !self.fluids[x][z][y]
                        .or_block_default(self.blocks[x][z][y])
                        .is_empty()
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn seed_water_fluids_from_blocks(&mut self) {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    if self.blocks[x][z][y] == BlockType::Water {
                        self.fluids[x][z][y] = FluidCell::water_source();
                    }
                }
            }
        }
    }

    fn decode_fluid_storage(&mut self, bytes: &[u8]) -> Result<(), ChunkDecodeError> {
        if bytes.len() < FLUID_STORAGE_MAGIC.len()
            || &bytes[..FLUID_STORAGE_MAGIC.len()] != FLUID_STORAGE_MAGIC
        {
            return Err(ChunkDecodeError::InvalidFluid);
        }
        let body = &bytes[FLUID_STORAGE_MAGIC.len()..];
        if body.len() < CHUNK_VOLUME {
            return Err(ChunkDecodeError::Truncated);
        }

        let mut pos = 0;
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    self.fluids[x][z][y] = FluidCell::from_storage_byte(body[pos])?;
                    pos += 1;
                }
            }
        }

        Ok(())
    }
}

impl FluidCell {
    fn or_block_default(self, block: BlockType) -> Self {
        if self.is_empty() {
            fluid_cell_for_block(block)
        } else {
            self
        }
    }
}

fn fluid_cell_for_block(block: BlockType) -> FluidCell {
    match block {
        BlockType::Water => FluidCell::water_source(),
        _ => FluidCell::default(),
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
    InvalidFluid,
    UnknownBlock(String),
}

impl std::fmt::Display for ChunkDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => write!(f, "chunk data truncated"),
            Self::InvalidHeader => write!(f, "invalid chunk header"),
            Self::InvalidFluid => write!(f, "invalid chunk fluid data"),
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
        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();
        assert_eq!(decoded, chunk);
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_all_air() {
        let chunk = Chunk::default();
        let bytes = chunk.to_storage_bytes();
        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();
        assert_eq!(decoded, chunk);
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_full_stone() {
        let mut chunk = Chunk::default();
        chunk.blocks = [[[BlockType::Stone; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];
        let bytes = chunk.to_storage_bytes();
        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();
        assert_eq!(decoded, chunk);
    }

    #[test]
    fn chunk_storage_bytes_roundtrip_water_fluid() {
        let mut chunk = Chunk::default();
        let pos = uvec3(2, 3, 4);
        chunk.set_block(pos, BlockType::Water);

        let bytes = chunk.to_storage_bytes();
        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();

        assert_eq!(decoded, chunk);
        assert_eq!(decoded.get_fluid(pos), FluidCell::water_source());
    }

    #[test]
    fn water_placement_seeds_fluid_cell_and_is_not_breakable() {
        let mut chunk = Chunk::default();
        let pos = uvec3(1, 2, 3);

        assert_eq!(
            chunk.place_block(pos, BlockType::Water),
            Some(BlockDelta {
                old: BlockType::Air,
                new: BlockType::Water,
            })
        );
        assert_eq!(chunk.get_fluid(pos), FluidCell::water_source());

        assert_eq!(chunk.break_block(pos), None);
        assert_eq!(chunk.get_block(pos), BlockType::Water);
        assert_eq!(chunk.get_fluid(pos), FluidCell::water_source());
    }

    #[test]
    fn solid_block_placement_replaces_water_and_clears_fluid_cell() {
        let mut chunk = Chunk::default();
        let pos = uvec3(1, 2, 3);
        chunk.place_block(pos, BlockType::Water).unwrap();

        assert_eq!(
            chunk.place_block(pos, BlockType::Stone),
            Some(BlockDelta {
                old: BlockType::Water,
                new: BlockType::Stone,
            })
        );
        assert_eq!(chunk.get_block(pos), BlockType::Stone);
        assert_eq!(chunk.get_fluid(pos), FluidCell::default());
    }

    #[test]
    fn old_storage_water_blocks_seed_fluid_cells() {
        let mut chunk = Chunk::default();
        let pos = uvec3(2, 3, 4);
        chunk.set_block(pos, BlockType::Water);
        let mut bytes = chunk.to_storage_bytes();
        let fluid_start = bytes
            .windows(FLUID_STORAGE_MAGIC.len())
            .position(|window| window == FLUID_STORAGE_MAGIC)
            .unwrap();
        bytes.truncate(fluid_start);

        let decoded = Chunk::try_from_storage_bytes(&bytes).unwrap();

        assert_eq!(decoded.get_block(pos), BlockType::Water);
        assert_eq!(decoded.get_fluid(pos), FluidCell::water_source());
    }

    #[test]
    fn chunk_storage_bytes_reject_garbled_data() {
        assert!(Chunk::try_from_storage_bytes(&[]).is_err());
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

        match Chunk::try_from_storage_bytes(&bytes) {
            Err(ChunkDecodeError::UnknownBlock(n)) => assert_eq!(n, "nonexistent"),
            other => panic!("expected UnknownBlock, got {other:?}"),
        }
    }
}
