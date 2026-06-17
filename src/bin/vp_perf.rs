use std::{hint::black_box, time::Instant};

use bevy::{platform::collections::HashMap, prelude::*};
use minecraft_clone::{
    block::BlockType,
    world::{
        WorldMetadata,
        chunk::mesh::{ChunkMeshBlocks, vertex_pulling},
        chunk::{CHUNK_SIZE, Chunk},
        generation::generate_chunk,
    },
};

struct Scenario {
    center_pos: IVec3,
    chunks: Vec<(IVec3, Chunk)>,
}

impl Scenario {
    fn chunk_refs(&self) -> HashMap<IVec3, &Chunk> {
        self.chunks.iter().map(|(p, c)| (*p, c)).collect()
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let scenario_name = args.next().unwrap_or_else(|| "checkerboard".to_owned());
    let iterations = args
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(250_000);

    let scenario = make_scenario(&scenario_name);
    let chunk_refs = scenario.chunk_refs();
    let start = Instant::now();
    let mut checksum = 0usize;

    for _ in 0..iterations {
        let blocks = ChunkMeshBlocks::from_chunks(scenario.center_pos, black_box(&chunk_refs));
        let layers = vertex_pulling::build_descriptors(black_box(&blocks));
        checksum = checksum.wrapping_add(
            layers
                .iter()
                .map(|(_, descriptors)| descriptors.len())
                .sum::<usize>(),
        );
        black_box(layers);
    }

    let elapsed = start.elapsed();
    println!(
        "scenario={scenario_name} iterations={iterations} checksum={checksum} elapsed={elapsed:?}"
    );
}

fn make_scenario(name: &str) -> Scenario {
    match name {
        "empty" => Scenario {
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, Chunk::default())],
        },
        "single_stone" => Scenario {
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, single_stone_chunk())],
        },
        "full_stone_buried" => Scenario {
            center_pos: IVec3::ZERO,
            chunks: filled_neighborhood(BlockType::Stone),
        },
        "full_stone_open" => Scenario {
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, filled_chunk(BlockType::Stone))],
        },
        "checkerboard" => Scenario {
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, checkerboard_chunk())],
        },
        "generated_surface" => generated_neighborhood_scenario(ivec3(0, 1, 0)),
        "realistic_terrain" => Scenario {
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, realistic_terrain_chunk())],
        },
        "realistic_terrain_buried" => realistic_terrain_buried_scenario(),
        _ => panic!(
            "unknown scenario {name:?}; expected empty, single_stone, full_stone_buried, full_stone_open, checkerboard, generated_surface, realistic_terrain, or realistic_terrain_buried"
        ),
    }
}

fn filled_chunk(block: BlockType) -> Chunk {
    Chunk {
        blocks: [[[block; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
    }
}

fn filled_neighborhood(block: BlockType) -> Vec<(IVec3, Chunk)> {
    let mut chunks = Vec::with_capacity(27);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                chunks.push((ivec3(x, y, z), filled_chunk(block)));
            }
        }
    }
    chunks
}

fn single_stone_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    chunk.blocks[8][8][8] = BlockType::Stone;
    chunk
}

fn checkerboard_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                if (x + y + z) % 2 == 0 {
                    chunk.blocks[x][z][y] = BlockType::Stone;
                }
            }
        }
    }
    chunk
}

fn generated_neighborhood_scenario(center_pos: IVec3) -> Scenario {
    let metadata = WorldMetadata::default();
    let mut chunks = Vec::with_capacity(27);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let pos = center_pos + ivec3(x, y, z);
                chunks.push((pos, generate_chunk(&metadata, pos)));
            }
        }
    }
    Scenario { center_pos, chunks }
}

fn realistic_terrain_buried_scenario() -> Scenario {
    let mut chunks = Vec::with_capacity(27);
    chunks.push((IVec3::ZERO, realistic_terrain_chunk()));
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if x != 0 || y != 0 || z != 0 {
                    chunks.push((ivec3(x, y, z), filled_chunk(BlockType::Stone)));
                }
            }
        }
    }
    Scenario {
        center_pos: IVec3::ZERO,
        chunks,
    }
}

fn realistic_terrain_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                chunk.blocks[x][z][y] = if y < 4 {
                    BlockType::Stone
                } else if y < 6 {
                    BlockType::Dirt
                } else if y == 6 {
                    BlockType::Grass
                } else if y >= 7 && y <= 11 && x >= 6 && x <= 9 && z >= 6 && z <= 9 {
                    BlockType::OakLog
                } else if y == 12
                    && x >= 5
                    && x <= 10
                    && z >= 5
                    && z <= 10
                    && !(x >= 7 && x <= 8 && z >= 7 && z <= 8)
                {
                    BlockType::OakLeaves
                } else if y == 11 && x >= 5 && x <= 10 && z >= 5 && z <= 10 {
                    BlockType::OakLeaves
                } else if y == 10
                    && x >= 5
                    && x <= 10
                    && z >= 5
                    && z <= 10
                    && (x == 5 || x == 10 || z == 5 || z == 10)
                {
                    BlockType::OakLeaves
                } else if y == 3 && (x + z) % 13 == 0 {
                    BlockType::Glass
                } else if y == 8 && (x * 7 + z * 11) % 23 == 0 {
                    BlockType::Glass
                } else {
                    BlockType::Air
                };
            }
        }
    }
    chunk
}
