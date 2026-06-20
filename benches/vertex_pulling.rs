//! Vertex-pulling mesh generation benchmark (8 scenarios).

use std::{hint::black_box, sync::Arc, time::Duration};

use bevy::{platform::collections::HashMap, prelude::*};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use minecraft_clone::{
    block::{BlockMaterialLayer, BlockType},
    world::{
        WorldMetadata,
        chunk::mesh::{ChunkMeshBlocks, vertex_pulling},
        chunk::{CHUNK_SIZE, Chunk, ChunkLight},
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

fn light_upload_lights(chunk_count: usize) -> Vec<(IVec3, ChunkLight)> {
    let edge = (chunk_count as f32).cbrt().ceil() as i32;
    let mut lights = Vec::with_capacity(chunk_count);
    for x in 0..edge {
        for y in 0..edge {
            for z in 0..edge {
                if lights.len() == chunk_count {
                    return lights;
                }
                let pos = ivec3(x, y, z);
                lights.push((pos, patterned_light(pos)));
            }
        }
    }
    lights
}

fn patterned_light(chunk_pos: IVec3) -> ChunkLight {
    let mut light = ChunkLight::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                let sky = (x as i32 * 3
                    + y as i32 * 5
                    + z as i32 * 7
                    + chunk_pos.x * 11
                    + chunk_pos.y * 13
                    + chunk_pos.z * 17) as u8;
                let block = (x as i32 * 7
                    + y as i32 * 3
                    + z as i32 * 5
                    + chunk_pos.x * 17
                    + chunk_pos.y * 11
                    + chunk_pos.z * 13) as u8;
                let pos = uvec3(x as u32, y as u32, z as u32);
                light.set_sky_light(pos, sky);
                light.set_block_light(pos, block);
            }
        }
    }
    light
}

// ---------------------------------------------------------------------------
// Benchmark
// ---------------------------------------------------------------------------

fn bench_vertex_pulling(c: &mut Criterion) {
    let scenarios = make_scenarios();

    bench_mesh_descriptors(c, &scenarios);
    print_data_size_summary(&scenarios);
    bench_light_upload(c);
}

fn bench_mesh_descriptors(c: &mut Criterion, scenarios: &[Scenario]) {
    let mut group = c.benchmark_group("vp_mesh");
    group.throughput(Throughput::Elements(1));
    for scenario in scenarios {
        let chunk_refs = scenario.chunk_refs();
        let center = scenario.center_pos;
        group.bench_function(BenchmarkId::from_parameter(scenario.name), |b| {
            b.iter(|| {
                let blocks = ChunkMeshBlocks::from_chunks(center, black_box(&chunk_refs));
                black_box(vertex_pulling::build_descriptors(&blocks))
            });
        });
    }
    group.finish();
}

fn print_data_size_summary(scenarios: &[Scenario]) {
    println!();
    println!("--- Data size comparison ---");
    println!("  {:<30} {:>8} {:>10}", "scenario", "vp faces", "vp desc",);
    for scenario in scenarios {
        let chunk_refs = scenario.chunk_refs();
        let blocks = ChunkMeshBlocks::from_chunks(scenario.center_pos, &chunk_refs);
        let vp_layers = vertex_pulling::build_descriptors(&blocks);
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

fn bench_light_upload(c: &mut Criterion) {
    let chunk_count = 4096;
    let layer_count = BlockMaterialLayer::COUNT;
    let lights = light_upload_lights(chunk_count);
    let light_refs = lights
        .iter()
        .map(|(pos, light)| (*pos, light))
        .collect::<HashMap<_, _>>();
    let positions = lights.iter().map(|(pos, _)| *pos).collect::<Vec<_>>();
    let light_blobs = positions
        .iter()
        .map(|pos| {
            let light_data: Arc<[u32]> =
                ChunkLight::build_padded_light_data(*pos, &light_refs).into();
            light_data
        })
        .collect::<Vec<_>>();
    let empty_light: Arc<[u32]> =
        ChunkLight::build_padded_light_data(IVec3::ZERO, &HashMap::default()).into();
    let mut components = (0..chunk_count * layer_count)
        .map(|_| vertex_pulling::VertexPullingLight {
            light_data: empty_light.clone(),
        })
        .collect::<Vec<_>>();

    let mut group = c.benchmark_group("vp_light_upload");
    group.throughput(Throughput::Elements(chunk_count as u64));
    group.bench_function("prebuilt_4096_chunks_all_layers", |b| {
        b.iter(|| {
            for (chunk_index, light_data) in light_blobs.iter().enumerate() {
                let first_child = chunk_index * layer_count;
                for layer_index in 0..layer_count {
                    components[first_child + layer_index].light_data = light_data.clone();
                }
            }
            black_box(&components);
        });
    });
    group.finish();
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
