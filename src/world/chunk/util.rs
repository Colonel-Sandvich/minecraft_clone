use crate::block::BlockType;

use super::{CHUNK_SIZE, Chunk};

pub fn generate_flat_chunk_data() -> Chunk {
    let mut chunk = Chunk::default();

    // Generate a flat floor of grass and glass blocks
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..=0 {
                chunk.blocks[x][z][y] = if (x + z) % 2 == 0 {
                    BlockType::Grass
                } else {
                    BlockType::Glass
                };
            }
        }
    }

    chunk
}

pub fn generate_full_chunk_data() -> Chunk {
    let mut blocks = [[[BlockType::Stone; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                blocks[x][z][y] = BlockType::random_not_air();
            }
        }
    }

    Chunk { blocks }
}
