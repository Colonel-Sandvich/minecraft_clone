use std::hint::black_box;
use std::time::Duration;

use bevy::{math::Rect, platform::collections::HashMap, prelude::*};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use minecraft_clone::{
    block::{BlockTextureMap, BlockType, block_and_side_to_texture_path},
    quad::Direction,
    world::{
        WorldMetadata,
        chunk::{
            CHUNK_SIZE, Chunk,
            ambient_occlusion::AmbientOcclusionSettings,
            mesh::{
                AdaptiveChunkMesher, ChunkMeshBlocks, ChunkMeshInput,
                ChunkMesher, DirectChunkMesher, GreedyChunkMesher, HybridChunkMesher,
                ReferenceChunkMesher, SweepChunkMesher, make_reference_layered_quad_groups,
            },
        },
        generation::generate_chunk,
    },
};
use strum::IntoEnumIterator;

struct ChunkMeshingScenario {
    name: &'static str,
    center_pos: IVec3,
    chunks: Vec<(IVec3, Chunk)>,
}

impl ChunkMeshingScenario {
    fn chunk_refs(&self) -> HashMap<IVec3, &Chunk> {
        self.chunks
            .iter()
            .map(|(pos, chunk)| (*pos, chunk))
            .collect()
    }
}

fn bench_chunk_meshing(c: &mut Criterion) {
    let texture_map = bench_texture_map();
    let ao_brightness = AmbientOcclusionSettings::default().brightness_curve();
    let scenarios = chunk_meshing_scenarios();

    let mut input_group = c.benchmark_group("chunk_mesh_input_build");
    input_group.throughput(Throughput::Elements(1));
    for scenario in &scenarios {
        let center_pos = scenario.center_pos;
        let chunk_refs = scenario.chunk_refs();
        input_group.bench_function(BenchmarkId::from_parameter(scenario.name), move |b| {
            b.iter(|| {
                black_box(ChunkMeshBlocks::from_chunks(
                    black_box(center_pos),
                    black_box(&chunk_refs),
                ))
            });
        });
    }
    input_group.finish();

    let inputs = scenarios
        .iter()
        .map(|scenario| {
            let chunk_refs = scenario.chunk_refs();
            (
                scenario.name,
                ChunkMeshBlocks::from_chunks(scenario.center_pos, &chunk_refs),
            )
        })
        .collect::<Vec<_>>();

    let mut quad_group = c.benchmark_group("reference_quad_groups");
    quad_group.throughput(Throughput::Elements(1));
    for (name, blocks) in &inputs {
        quad_group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| {
                black_box(make_reference_layered_quad_groups(
                    black_box(blocks),
                    black_box(&texture_map),
                ))
            });
        });
    }
    quad_group.finish();

    bench_mesher(
        c,
        "reference",
        ReferenceChunkMesher,
        &inputs,
        &texture_map,
        ao_brightness,
    );
    bench_mesher(
        c,
        "direct",
        DirectChunkMesher,
        &inputs,
        &texture_map,
        ao_brightness,
    );
    bench_mesher(
        c,
        "sweep",
        SweepChunkMesher,
        &inputs,
        &texture_map,
        ao_brightness,
    );
    bench_mesher(
        c,
        "hybrid",
        HybridChunkMesher,
        &inputs,
        &texture_map,
        ao_brightness,
    );
    bench_mesher(
        c,
        "greedy",
        GreedyChunkMesher,
        &inputs,
        &texture_map,
        ao_brightness,
    );
    bench_mesher(
        c,
        "adaptive",
        AdaptiveChunkMesher,
        &inputs,
        &texture_map,
        ao_brightness,
    );
}

fn bench_mesher(
    c: &mut Criterion,
    group_name: &'static str,
    mesher: impl ChunkMesher + Copy,
    inputs: &[(&'static str, ChunkMeshBlocks)],
    texture_map: &BlockTextureMap,
    ao_brightness: [f32; 4],
) {
    let mut mesh_group = c.benchmark_group(group_name);
    mesh_group.throughput(Throughput::Elements(1));
    for (name, blocks) in inputs {
        mesh_group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| {
                black_box(mesher.mesh(ChunkMeshInput {
                    blocks: black_box(blocks),
                    block_texture_map: black_box(texture_map),
                    ao_brightness: black_box(ao_brightness),
                }))
            });
        });
    }
    mesh_group.finish();
}

fn chunk_meshing_scenarios() -> Vec<ChunkMeshingScenario> {
    vec![
        ChunkMeshingScenario {
            name: "empty",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, Chunk::default())],
        },
        ChunkMeshingScenario {
            name: "single_stone",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, single_stone_chunk())],
        },
        ChunkMeshingScenario {
            name: "sparse_stone",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, sparse_stone_chunk())],
        },
        ChunkMeshingScenario {
            name: "full_stone_open",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, filled_chunk(BlockType::Stone))],
        },
        ChunkMeshingScenario {
            name: "full_stone_buried",
            center_pos: IVec3::ZERO,
            chunks: filled_neighborhood(BlockType::Stone),
        },
        ChunkMeshingScenario {
            name: "checkerboard_stone",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, checkerboard_chunk())],
        },
        ChunkMeshingScenario {
            name: "dense_leaves",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, filled_chunk(BlockType::OakLeaves))],
        },
        ChunkMeshingScenario {
            name: "mixed_transparency",
            center_pos: IVec3::ZERO,
            chunks: vec![(IVec3::ZERO, mixed_transparency_chunk())],
        },
        generated_neighborhood_scenario("generated_underground", ivec3(0, 0, 0)),
        generated_neighborhood_scenario("generated_surface", ivec3(0, 1, 0)),
        realistic_terrain_scenario(),
    ]
}

fn bench_texture_map() -> BlockTextureMap {
    let mut paths = HashMap::default();

    for block in BlockType::iter() {
        if block == BlockType::Air {
            continue;
        }

        for side in Direction::iter() {
            let path = block_and_side_to_texture_path(block, side);
            let next = paths.len() as f32;
            paths.entry(path.to_owned()).or_insert_with(|| {
                let min = next * 0.01;
                Rect::new(min, min, min + 0.005, min + 0.005)
            });
        }
    }

    BlockTextureMap(paths)
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

fn sparse_stone_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                if (x * 7_349 + y * 9_151 + z * 1_237) % 37 == 0 {
                    chunk.blocks[x][z][y] = BlockType::Stone;
                }
            }
        }
    }
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

fn mixed_transparency_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                chunk.blocks[x][z][y] = if y < 5 {
                    BlockType::Stone
                } else if (x + z) % 7 == 0 {
                    BlockType::Glass
                } else if (x * 3 + y + z * 5) % 11 == 0 {
                    BlockType::OakLeaves
                } else {
                    BlockType::Air
                };
            }
        }
    }
    chunk
}

fn generated_neighborhood_scenario(name: &'static str, center_pos: IVec3) -> ChunkMeshingScenario {
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

    ChunkMeshingScenario {
        name,
        center_pos,
        chunks,
    }
}

fn realistic_terrain_scenario() -> ChunkMeshingScenario {
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
                } else if y == 12 && x >= 5 && x <= 10 && z >= 5 && z <= 10
                    && !(x >= 7 && x <= 8 && z >= 7 && z <= 8)
                {
                    BlockType::OakLeaves
                } else if y == 11 && x >= 5 && x <= 10 && z >= 5 && z <= 10 {
                    BlockType::OakLeaves
                } else if y == 10 && x >= 5 && x <= 10 && z >= 5 && z <= 10
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

    ChunkMeshingScenario {
        name: "realistic_terrain",
        center_pos: IVec3::ZERO,
        chunks: vec![(IVec3::ZERO, chunk)],
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(2))
        .sample_size(15);
    targets = bench_chunk_meshing
}
criterion_main!(benches);
