use std::collections::HashMap;

use super::{
    coords::{CHUNK_SIZE, CHUNK_VOLUME, ChunkIndex, chunk_linear_index},
    data::Chunk,
    state::ChunkCell,
};

impl Chunk {
    /// Encode this chunk with semantic palette names and MSB-first packed indices.
    ///
    /// A chunk without pending fluid ticks encodes exactly like the
    /// pre-scheduling format; see [`Self::to_storage_bytes_with_scheduled`].
    pub fn to_storage_bytes(&self) -> Vec<u8> {
        self.to_storage_bytes_with_scheduled(&[])
    }

    /// Encode this chunk plus its pending fluid ticks.
    ///
    /// Pending ticks are chunk-local [`ChunkIndex`] positions appended after the
    /// packed body as `[u16 count][u16 index; count]` (little-endian), sorted and
    /// deduplicated for a deterministic encoding. The section is omitted when
    /// there are no pending ticks.
    pub fn to_storage_bytes_with_scheduled(&self, scheduled: &[ChunkIndex]) -> Vec<u8> {
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

        if !scheduled.is_empty() {
            let mut sorted = scheduled.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            bytes.extend_from_slice(&(sorted.len() as u16).to_le_bytes());
            for index in sorted {
                bytes.extend_from_slice(&(index.as_usize() as u16).to_le_bytes());
            }
        }

        bytes
    }

    /// Decodes one chunk, rejecting blobs that carry pending fluid ticks.
    ///
    /// Persistence boundaries should decode through
    /// [`Self::try_from_storage_bytes_with_scheduled`] so restored ticks are
    /// not silently discarded.
    pub fn try_from_storage_bytes(bytes: &[u8]) -> Result<Self, ChunkDecodeError> {
        let (chunk, scheduled) = Self::try_from_storage_bytes_with_scheduled(bytes)?;
        if scheduled.is_empty() {
            Ok(chunk)
        } else {
            Err(ChunkDecodeError::InvalidHeader)
        }
    }

    /// Decodes one chunk plus its optional trailing pending-tick section.
    pub fn try_from_storage_bytes_with_scheduled(
        bytes: &[u8],
    ) -> Result<(Self, Vec<ChunkIndex>), ChunkDecodeError> {
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
        if pos > bytes.len() {
            return Err(ChunkDecodeError::Truncated);
        }

        let mut scheduled = Vec::new();
        if pos < bytes.len() {
            let trailer = &bytes[pos..];
            let count = trailer
                .get(..2)
                .map(|count| u16::from_le_bytes([count[0], count[1]]) as usize)
                .ok_or(ChunkDecodeError::InvalidHeader)?;
            if trailer.len() != 2 + count * 2 {
                return Err(ChunkDecodeError::InvalidHeader);
            }
            for index in 0..count {
                let offset = 2 + index * 2;
                let raw = u16::from_le_bytes([trailer[offset], trailer[offset + 1]]);
                let index = ChunkIndex::try_from_usize(raw as usize)
                    .ok_or(ChunkDecodeError::InvalidHeader)?;
                scheduled.push(index);
            }
        }

        Ok((chunk, scheduled))
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
    use crate::item::Item;
    use crate::world::chunk::LocalBlockPos;

    #[test]
    fn representative_encoding_has_stable_wire_fingerprint() {
        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, Item::Grass.into());
        chunk.set_cell_xyz(1, 0, 0, Item::Dirt.into());
        chunk.set_cell_xyz(0, 0, 1, Item::Stone.into());
        chunk.set_cell_xyz(15, 15, 15, ChunkCell::water_source());

        let bytes = chunk.to_storage_bytes_with_scheduled(&[
            LocalBlockPos::new(15, 15, 15).index(),
            LocalBlockPos::new(0, 8, 3).index(),
        ]);
        let fingerprint = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
        });

        assert_eq!(bytes.len(), 1_581);
        assert_eq!(fingerprint, 0x4584_c82a_691d_4c13);
    }

    #[test]
    fn pending_ticks_round_trip_through_the_trailing_section() {
        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(4, 5, 6, ChunkCell::water_source());

        let ticks = vec![
            LocalBlockPos::new(7, 1, 2).index(),
            LocalBlockPos::new(4, 5, 6).index(),
            LocalBlockPos::new(7, 1, 2).index(),
        ];
        let bytes = chunk.to_storage_bytes_with_scheduled(&ticks);
        let (decoded, scheduled) = Chunk::try_from_storage_bytes_with_scheduled(&bytes).unwrap();

        assert_eq!(decoded, chunk);
        assert_eq!(
            scheduled,
            vec![
                LocalBlockPos::new(4, 5, 6).index(),
                LocalBlockPos::new(7, 1, 2).index(),
            ]
        );
        assert!(
            Chunk::try_from_storage_bytes(&bytes).is_err(),
            "content-only decode must reject blobs that still carry pending ticks"
        );
    }

    #[test]
    fn settled_chunks_stay_byte_identical_and_decode_without_ticks() {
        let mut chunk = Chunk::default();
        chunk.set_cell_xyz(0, 0, 0, Item::Stone.into());

        let plain = chunk.to_storage_bytes();
        assert_eq!(chunk.to_storage_bytes_with_scheduled(&[]), plain);

        let (decoded, scheduled) = Chunk::try_from_storage_bytes_with_scheduled(&plain).unwrap();
        assert_eq!(decoded, chunk);
        assert!(scheduled.is_empty());
    }

    #[test]
    fn malformed_pending_tick_sections_are_rejected() {
        let chunk = Chunk::default();
        let bytes = chunk.to_storage_bytes();

        let mut wrong_count = bytes.clone();
        wrong_count.extend_from_slice(&2u16.to_le_bytes());
        wrong_count.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            Chunk::try_from_storage_bytes_with_scheduled(&wrong_count),
            Err(ChunkDecodeError::InvalidHeader)
        );

        let mut out_of_range = bytes.clone();
        out_of_range.extend_from_slice(&1u16.to_le_bytes());
        out_of_range.extend_from_slice(&(CHUNK_VOLUME as u16).to_le_bytes());
        assert_eq!(
            Chunk::try_from_storage_bytes_with_scheduled(&out_of_range),
            Err(ChunkDecodeError::InvalidHeader)
        );

        let mut missing_index = bytes.clone();
        missing_index.extend_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            Chunk::try_from_storage_bytes_with_scheduled(&missing_index),
            Err(ChunkDecodeError::InvalidHeader)
        );
    }
}
