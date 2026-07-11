use std::collections::HashMap;

use super::{
    coords::{CHUNK_SIZE, CHUNK_VOLUME, chunk_linear_index},
    data::Chunk,
    state::ChunkCell,
};

impl Chunk {
    /// Encode this chunk with semantic palette names and MSB-first packed indices.
    pub fn to_storage_bytes(&self) -> Vec<u8> {
        let palette = self.storage_palette();
        let bits = bits_for(palette.len());
        let cell_to_idx: HashMap<ChunkCell, u32> = palette
            .iter()
            .enumerate()
            .map(|(index, &cell)| (cell, index as u32))
            .collect();
        let indices = self
            .iter()
            .map(|(cell, _)| cell_to_idx[&cell])
            .collect::<Vec<_>>();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(palette.len() as u16).to_le_bytes());
        for &cell in &palette {
            let name = cell.name();
            bytes.push(name.len() as u8);
            bytes.extend_from_slice(name.as_bytes());
        }
        bytes.push(bits);

        let body_start = bytes.len();
        let body_bytes = (indices.len() * bits as usize).div_ceil(8);
        bytes.resize(body_start + body_bytes, 0);
        pack(&mut bytes[body_start..], &indices, bits);
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
            let cell = ChunkCell::from_name(name)
                .ok_or_else(|| ChunkDecodeError::UnknownBlock(name.to_owned()))?;
            palette.push(cell);
        }

        let bits = *bytes.get(pos).ok_or(ChunkDecodeError::Truncated)?;
        if bits == 0 || bits > 32 {
            return Err(ChunkDecodeError::InvalidHeader);
        }
        pos += 1;

        let body = bytes.get(pos..).ok_or(ChunkDecodeError::Truncated)?;
        let mask = if bits == 32 {
            u32::MAX
        } else {
            (1u32 << bits) - 1
        };
        let body_bytes = (CHUNK_VOLUME * bits as usize).div_ceil(8);

        let mut chunk = Chunk::default();
        let mut bit_pos = 0usize;
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let index =
                        read_bits(body, &mut bit_pos, bits).ok_or(ChunkDecodeError::Truncated)?;
                    let index = (index & mask) as usize;
                    if index >= palette.len() {
                        return Err(ChunkDecodeError::InvalidHeader);
                    }
                    chunk.write_cell_linear(chunk_linear_index(x, y, z), palette[index]);
                }
            }
        }

        pos += body_bytes;
        if pos != bytes.len() {
            return Err(ChunkDecodeError::InvalidHeader);
        }

        Ok(chunk)
    }
}

fn bits_for(palette_size: usize) -> u8 {
    match palette_size {
        0 | 1 => 1,
        count => (usize::BITS - (count - 1).leading_zeros()) as u8,
    }
}

#[inline]
fn pack(buffer: &mut [u8], indices: &[u32], bits: u8) {
    let mut bit_pos = 0usize;
    for &index in indices {
        let mut value = index;
        for _ in 0..bits {
            buffer[bit_pos >> 3] |= (((value >> (bits - 1)) & 1) as u8) << (7 - (bit_pos & 7));
            value <<= 1;
            bit_pos += 1;
        }
    }
}

#[inline]
fn read_bits(buffer: &[u8], bit_pos: &mut usize, bits: u8) -> Option<u32> {
    let mut value = 0u32;
    for _ in 0..bits {
        let byte = buffer.get(*bit_pos >> 3)?;
        let bit = (byte >> (7 - (*bit_pos & 7))) & 1;
        value = (value << 1) | bit as u32;
        *bit_pos += 1;
    }
    Some(value)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;

    #[test]
    fn representative_encoding_has_stable_wire_fingerprint() {
        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, BlockType::Grass.into());
        chunk.set_cell_xyz(1, 0, 0, BlockType::Dirt.into());
        chunk.set_cell_xyz(0, 0, 1, BlockType::Stone.into());
        chunk.set_cell_xyz(15, 15, 15, ChunkCell::water_source());

        let bytes = chunk.to_storage_bytes();
        let fingerprint = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
        });

        assert_eq!(bytes.len(), 1_575);
        assert_eq!(fingerprint, 0x63fa_5f04_4acf_91df);
    }
}
