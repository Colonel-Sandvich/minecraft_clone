//! Benchmarks: vertex-pulling vs greedy meshing head-to-head.
//!
//! Each path is measured end-to-end from dirty chunk → GPU-ready output.
//! VP: ChunkMeshBlocks::from_chunks + build_descriptors
//! Greedy: ChunkMeshBlocks::from_chunks + GreedyChunkMesher::mesh
//! Light buffer excluded — it's a light-system concern, not mesh generation.

use std::hint::black_box;
use std::time::Duration;

use bevy::{math::Rect, platform::collections::HashMap, prelude::*};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use minecraft_clone::{
    block::{BlockMaterialLayer, BlockTextureMap, BlockType, block_and_side_to_texture_path},
    quad::Direction,
    world::{
        WorldMetadata,
        chunk::{
            CHUNK_SIZE, Chunk, ChunkLight,
            ambient_occlusion::AmbientOcclusionSettings,
            mesh::{
                ChunkMeshBlocks, ChunkMeshInput, ChunkMesher, GreedyChunkMesher, vertex_pulling,
            },
        },
        generation::generate_chunk,
    },
};
use strum::IntoEnumIterator;

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
        // realistic_terrain with neighbors (proper occlusion)
        realistic_terrain_buried_scenario("realistic_terrain_buried"),
    ]
}

fn realistic_terrain_buried_scenario(name: &'static str) -> Scenario {
    let center = realistic_terrain_chunk();
    let mut chunks = Vec::with_capacity(27);
    // Center
    chunks.push((IVec3::ZERO, center));
    // 26 neighbors: all filled with stone (provides occlusion)
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
// Chunk helpers (mirroring chunk_meshing.rs)
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
// Texture map
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Data size helpers
// ---------------------------------------------------------------------------

const FACEDESCRIPTOR_BYTES: usize = std::mem::size_of::<vertex_pulling::FaceDescriptor>();

fn vp_data_bytes(
    layers: &[(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)],
    light_data: &[u32],
) -> usize {
    let descriptor_bytes: usize = layers
        .iter()
        .map(|(_, desc)| desc.len() * FACEDESCRIPTOR_BYTES)
        .sum();
    let light_bytes = light_data.len() * std::mem::size_of::<u32>();
    descriptor_bytes + light_bytes
}

fn vp_face_count(layers: &[(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)]) -> usize {
    layers.iter().map(|(_, desc)| desc.len()).sum()
}

fn greedy_data_bytes(meshes: &[(BlockMaterialLayer, Mesh)]) -> usize {
    let mut total = 0usize;
    for (_, mesh) in meshes {
        total += mesh.get_vertex_buffer_size();
        if let Some(index_bytes) = mesh.get_index_buffer_bytes() {
            total += index_bytes.len();
        }
    }
    total
}

fn greedy_face_count(meshes: &[(BlockMaterialLayer, Mesh)]) -> usize {
    let mut total = 0usize;
    for (_, mesh) in meshes {
        if let Some(indices) = mesh.indices() {
            total += match indices {
                bevy::mesh::Indices::U16(raw) => raw.len(),
                bevy::mesh::Indices::U32(raw) => raw.len(),
            } / 3;
        }
    }
    total
}

// ---------------------------------------------------------------------------
// Benchmark
// ---------------------------------------------------------------------------

fn bench_vertex_pulling(c: &mut Criterion) {
    let texture_map = bench_texture_map();
    let ao_brightness = AmbientOcclusionSettings::default().brightness_curve();
    let scenarios = make_scenarios();

    let lights_by_pos: HashMap<IVec3, &ChunkLight> = HashMap::new();

    // Build ChunkMeshBlocks once per scenario
    let inputs: Vec<(&Scenario, ChunkMeshBlocks)> = scenarios
        .iter()
        .map(|s| {
            let chunk_refs = s.chunk_refs();
            (s, ChunkMeshBlocks::from_chunks(s.center_pos, &chunk_refs))
        })
        .collect();

    // ------- vp_mesh: ChunkMeshBlocks + build_descriptors (end-to-end VP pipeline) -------
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

    // ------- greedy_mesh: ChunkMeshBlocks + GreedyChunkMesher (end-to-end greedy pipeline) -------
    {
        let mut group = c.benchmark_group("greedy_mesh");
        group.throughput(Throughput::Elements(1));
        for (scenario, _blocks) in &inputs {
            let chunk_refs = scenario.chunk_refs();
            let center = scenario.center_pos;
            group.bench_function(BenchmarkId::from_parameter(scenario.name), |b| {
                b.iter(|| {
                    let blocks = ChunkMeshBlocks::from_chunks(center, &chunk_refs);
                    let meshes = GreedyChunkMesher.mesh(ChunkMeshInput {
                        blocks: &blocks,
                        block_texture_map: &texture_map,
                        ao_brightness,
                    });
                    black_box(meshes)
                });
            });
        }
        group.finish();
    }

    // ------- Print data-size comparison per scenario -------
    println!();
    println!("--- Data size comparison ---");
    println!(
        "  {:<30} {:>8} {:>8} {:>10} {:>12} {:>12}",
        "scenario", "vp faces", "gr faces", "vp desc", "vp+light", "gr bytes",
    );
    for (scenario, blocks) in &inputs {
        let vp_layers = vertex_pulling::build_descriptors(blocks);
        let light_data = ChunkLight::build_padded_light_data(scenario.center_pos, &lights_by_pos);
        let vp_desc_bytes: usize = vp_layers
            .iter()
            .map(|(_, desc)| desc.len() * FACEDESCRIPTOR_BYTES)
            .sum();
        let vp_total = vp_data_bytes(&vp_layers, &light_data);
        let vp_faces = vp_face_count(&vp_layers);

        let greedy_meshes = GreedyChunkMesher.mesh(ChunkMeshInput {
            blocks,
            block_texture_map: &texture_map,
            ao_brightness,
        });
        let greedy_bytes = greedy_data_bytes(&greedy_meshes);
        let greedy_faces = greedy_face_count(&greedy_meshes);

        println!(
            "  {:<30} {:>8} {:>8} {:>10} {:>12} {:>12}",
            scenario.name, vp_faces, greedy_faces, vp_desc_bytes, vp_total, greedy_bytes,
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
