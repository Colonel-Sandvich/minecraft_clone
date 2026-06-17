//! Vertex-pulling mesh generation benchmark (8 scenarios).

use std::hint::black_box;
use std::time::Duration;

use bevy::{platform::collections::HashMap, prelude::*};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use minecraft_clone::{
    block::{BlockMaterialLayer, BlockType},
    world::{
        WorldMetadata,
        chunk::mesh::{ChunkMeshBlocks, vertex_pulling},
        chunk::{CHUNK_SIZE, Chunk},
        generation::generate_chunk,
    },
};

// ---------------------------------------------------------------------------
// Bench scenario
// ---------------------------------------------------------------------------

struct Scenario {
    name: &'static str,
    center_pos: IVec3,
    chunks: Vec<(IVec3, Chunk)>,
}

impl Scenario {
    fn chunk_refs(&self) -> HashMap<IVec3, &Chunk> {
        self.chunks.iter().map(|(p, c)| (*p, c)).collect()
    }
}

fn make_scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "empty",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, Chunk::default())],
        },
        Scenario {
            name: "single_stone",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, single_stone_chunk())],
        },
        Scenario {
            name: "full_stone_buried",
            center_pos: IVec3::ZERO,
            chunks: filled_neighborhood(BlockType::Stone),
        },
        Scenario {
            name: "full_stone_open",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, filled_chunk(BlockType::Stone))],
        },
        Scenario {
            name: "checkerboard",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, checkerboard_chunk())],
        },
        generated_neighborhood_scenario("generated_surface", ivec3(0, 1, 0)),
        Scenario {
            name: "realistic_terrain",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, realistic_terrain_chunk())],
        },
        realistic_terrain_buried_scenario("realistic_terrain_buried"),
    ]
}

fn realistic_terrain_buried_scenario(name: &'static str) -> Scenario {
    let center = realistic_terrain_chunk();
    let mut chunks = Vec::with_capacity(27);
    chunks.push((IVec3::ZERO, center));
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if x == 0 && y == 0 && z == 0 {
                    continue;
                }
                chunks.push((ivec3(x, y, z), filled_chunk(BlockType::Stone)));
            }
        }
    }
    Scenario {
        name,
        center_pos: IVec3::ZERO,
        chunks,
    }
}

// ---------------------------------------------------------------------------
// Chunk helpers
// ---------------------------------------------------------------------------

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

fn generated_neighborhood_scenario(name: &'static str, center_pos: IVec3) -> Scenario {
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
    Scenario {
        name,
        center_pos,
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

// ---------------------------------------------------------------------------
// Data size helpers
// ---------------------------------------------------------------------------

const FACEDESCRIPTOR_BYTES: usize = std::mem::size_of::<vertex_pulling::FaceDescriptor>();

fn vp_face_count(layers: &[(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)]) -> usize {
    layers.iter().map(|(_, desc)| desc.len()).sum()
}

// ---------------------------------------------------------------------------
// Benchmark
// ---------------------------------------------------------------------------

fn bench_vertex_pulling(c: &mut Criterion) {
    let scenarios = make_scenarios();

    let inputs: Vec<(&Scenario, ChunkMeshBlocks)> = scenarios
        .iter()
        .map(|s| {
            let chunk_refs = s.chunk_refs();
            (s, ChunkMeshBlocks::from_chunks(s.center_pos, &chunk_refs))
        })
        .collect();

    // ------- vp_mesh: ChunkMeshBlocks + build_descriptors -------
    {
        let mut group = c.benchmark_group("vp_mesh");
        group.throughput(Throughput::Elements(1));
        for (scenario, _blocks) in &inputs {
            let chunk_refs = scenario.chunk_refs();
            let center = scenario.center_pos;
            group.bench_function(BenchmarkId::from_parameter(scenario.name), |b| {
                b.iter(|| {
                    let blocks = ChunkMeshBlocks::from_chunks(center, &chunk_refs);
                    black_box(vertex_pulling::build_descriptors(&blocks))
                });
            });
        }
        group.finish();
    }

    // ------- Data-size summary -------
    println!();
    println!("--- Data size comparison ---");
    println!("  {:<30} {:>8} {:>10}", "scenario", "vp faces", "vp desc",);
    for (scenario, blocks) in &inputs {
        let vp_layers = vertex_pulling::build_descriptors(blocks);
        let vp_desc_bytes: usize = vp_layers
            .iter()
            .map(|(_, desc)| desc.len() * FACEDESCRIPTOR_BYTES)
            .sum();
        let vp_faces = vp_face_count(&vp_layers);

        println!(
            "  {:<30} {:>8} {:>10}",
            scenario.name, vp_faces, vp_desc_bytes,
        );
    }
    println!();
}

criterion_group! {
    name = vp_benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(10);
    targets = bench_vertex_pulling
}
criterion_main!(vp_benches);
