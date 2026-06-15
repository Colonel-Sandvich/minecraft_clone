use std::hint::black_box;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use minecraft_clone::{
    block::BlockType,
    world::{
        WorldMetadata,
        chunk::{
            light::{ChunkHeightmap, ChunkLight, compute_light, light_on_place_sky, light_on_place_block},
            CHUNK_SIZE, CHUNK_VOLUME, Chunk,
        },
        generation::generate_chunk,
    },
};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn empty_chunk() -> Chunk {
    Chunk::default()
}

fn solid_chunk(block: BlockType) -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                chunk.blocks[x][z][y] = block;
            }
        }
    }
    chunk
}

fn surface_terrain_chunk() -> Chunk {
    let metadata = WorldMetadata::default();
    generate_chunk(&metadata, IVec3::new(1, 0, 1))
}

fn checkerboard_leaves_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                if (x + y + z) % 2 == 0 {
                    chunk.blocks[x][z][y] = BlockType::OakLeaves;
                }
            }
        }
    }
    chunk
}

fn glowstone_lattice_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                if x % 4 == 0 && z % 4 == 0 && y % 4 == 0 {
                    chunk.blocks[x][z][y] = BlockType::Glowstone;
                }
            }
        }
    }
    chunk
}

fn hollow_chamber_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                let is_surface = x == 0 || x == CHUNK_SIZE - 1
                    || z == 0 || z == CHUNK_SIZE - 1
                    || y == 0 || y == CHUNK_SIZE - 1;
                if is_surface {
                    chunk.blocks[x][z][y] = BlockType::Stone;
                }
            }
        }
    }
    chunk
}

fn cave_chunk() -> Chunk {
    let mut chunk = solid_chunk(BlockType::Stone);
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                let cx = (x as f32 - 7.5).powi(2);
                let cz = (z as f32 - 7.5).powi(2);
                let dist = (cx + cz).sqrt();
                let wave = ((z as f32 * 1.5).sin() * 3.0 + 8.0) as i32;
                if dist < 5.5 && (y as i32 - wave).abs() < 3 {
                    chunk.blocks[x][z][y] = BlockType::Air;
                }
                let wave2 = ((x as f32 * 1.3).cos() * 3.0 + 6.0) as i32;
                if (y as i32 - wave2).abs() < 2 && z > 2 && z < 13 && x > 1 && x < 14 {
                    chunk.blocks[x][z][y] = BlockType::Air;
                }
            }
        }
    }
    chunk.blocks[4][7][8] = BlockType::Glowstone;
    chunk.blocks[11][10][8] = BlockType::Glowstone;
    chunk
}

fn glass_column_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            chunk.blocks[x][z][0] = BlockType::Stone;
        }
    }
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 1..CHUNK_SIZE {
                if x % 3 == 0 && z % 3 == 0 {
                    chunk.blocks[x][z][y] = if (y + x / 3 + z / 3) % 2 == 0 {
                        BlockType::Glass
                    } else {
                        BlockType::OakLeaves
                    };
                }
            }
        }
    }
    chunk
}

fn full_neighborhood() -> HashMap<IVec3, (Chunk, ChunkLight)> {
    let mut m = HashMap::new();
    let metadata = WorldMetadata::default();
    for dx in -1..=1i32 {
        for dz in -1..=1i32 {
            for dy in -1..=1i32 {
                let pos = IVec3::new(dx, dy, dz);
                if pos == IVec3::ZERO {
                    continue;
                }
                let chunk = generate_chunk(&metadata, IVec3::new(1 + dx, dy, 1 + dz));
                m.insert(pos, (chunk, ChunkLight::default()));
            }
        }
    }
    m
}

// ── Full compute benchmark ───────────────────────────────────────────────────

struct FullScenario {
    name: &'static str,
    center_chunk: Chunk,
    rendered: u16,
    neighbors: HashMap<IVec3, (Chunk, ChunkLight)>,
}

fn full_scenarios() -> Vec<FullScenario> {
    vec![
        FullScenario { name: "empty", center_chunk: empty_chunk(), rendered: 0, neighbors: HashMap::new() },
        FullScenario { name: "solid_stone", center_chunk: solid_chunk(BlockType::Stone), rendered: CHUNK_VOLUME as u16, neighbors: HashMap::new() },
        FullScenario { name: "surface_terrain", center_chunk: surface_terrain_chunk(), rendered: count_rendered_blocks(&surface_terrain_chunk()), neighbors: HashMap::new() },
        FullScenario { name: "checkerboard_leaves", center_chunk: checkerboard_leaves_chunk(), rendered: count_rendered_blocks(&checkerboard_leaves_chunk()), neighbors: HashMap::new() },
        FullScenario { name: "hollow_chamber", center_chunk: hollow_chamber_chunk(), rendered: count_rendered_blocks(&hollow_chamber_chunk()), neighbors: HashMap::new() },
        FullScenario { name: "cave", center_chunk: cave_chunk(), rendered: count_rendered_blocks(&cave_chunk()), neighbors: HashMap::new() },
        FullScenario { name: "glass_column", center_chunk: glass_column_chunk(), rendered: count_rendered_blocks(&glass_column_chunk()), neighbors: HashMap::new() },
        FullScenario { name: "glowstone_lattice", center_chunk: glowstone_lattice_chunk(), rendered: count_rendered_blocks(&glowstone_lattice_chunk()), neighbors: HashMap::new() },
        FullScenario { name: "surface_terrain_cross", center_chunk: surface_terrain_chunk(), rendered: count_rendered_blocks(&surface_terrain_chunk()), neighbors: full_neighborhood() },
    ]
}

fn count_rendered_blocks(chunk: &Chunk) -> u16 {
    let mut count = 0u16;
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                if chunk.blocks[x][z][y].is_rendered() {
                    count += 1;
                }
            }
        }
    }
    count
}

fn bench_full_light_compute(c: &mut Criterion) {
    let scenarios = full_scenarios();
    let mut group = c.benchmark_group("light_full_compute");
    group.throughput(Throughput::Elements(1));

    for scenario in &scenarios {
        // Pre-build block refs (immutable, shared across iterations)
        let blocks: HashMap<IVec3, &Chunk> = scenario.neighbors.iter()
            .map(|(pos, (chunk, _light))| (*pos, chunk))
            .collect();

        // Template for neighbor lights (cloned fresh each iteration)
        let neighbor_template: Vec<(IVec3, ChunkLight)> = scenario.neighbors.iter()
            .map(|(pos, (_, light))| (*pos, light.clone()))
            .collect();

        group.bench_function(BenchmarkId::from_parameter(scenario.name), move |b| {
            b.iter(|| {
                let mut center_light = ChunkLight::default();
                let mut heightmap = ChunkHeightmap::default();
                let mut neighbor_lights: Vec<(IVec3, ChunkLight)> = neighbor_template.clone();
                let mut lights_map: HashMap<IVec3, &mut ChunkLight> = neighbor_lights.iter_mut()
                    .map(|(pos, light)| (*pos, light))
                    .collect();
                let mut dirty = 0;
                compute_light(
                    black_box(&scenario.center_chunk),
                    black_box(&mut center_light),
                    black_box(&mut heightmap),
                    black_box(&blocks),
                    black_box(&mut lights_map),
                    &mut dirty,
                    scenario.rendered,
                    0,
                    false,
                );
            });
        });
    }
}

struct IncrementalScenario {
    name: &'static str,
    center_chunk: Chunk,
    rendered: u16,
    neighbors: HashMap<IVec3, (Chunk, ChunkLight)>,
    /// World-space position in the center chunk to place/break
    place_pos: IVec3,
}

fn incremental_scenarios() -> Vec<IncrementalScenario> {
    vec![
        // Place stone in middle of empty chunk -> sky decrease
        IncrementalScenario {
            name: "place_stone_empty",
            center_chunk: empty_chunk(),
            rendered: 0,
            neighbors: HashMap::new(),
            place_pos: IVec3::new(8, 8, 8),
        },
        // Place glass in solid chunk -> no sky through, block light only
        IncrementalScenario {
            name: "place_glass_solid",
            center_chunk: solid_chunk(BlockType::Stone),
            rendered: CHUNK_VOLUME as u16,
            neighbors: HashMap::new(),
            place_pos: IVec3::new(8, 8, 8),
        },
        // Place glowstone -> triggers block light increase
        IncrementalScenario {
            name: "place_glowstone",
            center_chunk: empty_chunk(),
            rendered: 0,
            neighbors: HashMap::new(),
            place_pos: IVec3::new(8, 8, 8),
        },
    ]
}

fn bench_incremental_light(c: &mut Criterion) {
    let scenarios = incremental_scenarios();
    let mut sky_group = c.benchmark_group("light_incremental_sky");
    sky_group.throughput(Throughput::Elements(1));

    for scenario in &scenarios {
        let blocks: HashMap<IVec3, &Chunk> = scenario.neighbors.iter()
            .map(|(pos, (chunk, _))| (*pos, chunk))
            .collect();
        let neighbor_template: Vec<(IVec3, ChunkLight)> = scenario.neighbors.iter()
            .map(|(pos, (_, light))| (*pos, light.clone()))
            .collect();

        // Sky decrease (place)
        sky_group.bench_function(BenchmarkId::from_parameter(scenario.name), move |b| {
            b.iter(|| {
                let mut center_light = ChunkLight::default();
                // First do a full compute to establish baseline lighting
                let mut heightmap = ChunkHeightmap::default();
                let mut neighbor_lights: Vec<(IVec3, ChunkLight)> = neighbor_template.clone();
                let mut lights_map: HashMap<IVec3, &mut ChunkLight> = neighbor_lights.iter_mut()
                    .map(|(pos, light)| (*pos, light))
                    .collect();
                let mut dirty = 0;
                compute_light(
                    black_box(&scenario.center_chunk),
                    black_box(&mut center_light),
                    black_box(&mut heightmap),
                    black_box(&blocks),
                    black_box(&mut lights_map),
                    &mut dirty,
                    scenario.rendered,
                    0,
                    false,
                );
                // Now place and run incremental
                light_on_place_sky(
                    black_box(&scenario.center_chunk),
                    black_box(&mut center_light),
                    black_box(&blocks),
                    black_box(&mut lights_map),
                    black_box(scenario.place_pos),
                    &mut dirty,
                );
            });
        });
    }

    sky_group.finish();

    let mut block_group = c.benchmark_group("light_incremental_block");
    block_group.throughput(Throughput::Elements(1));

    for scenario in &scenarios {
        let blocks: HashMap<IVec3, &Chunk> = scenario.neighbors.iter()
            .map(|(pos, (chunk, _))| (*pos, chunk))
            .collect();
        let neighbor_template: Vec<(IVec3, ChunkLight)> = scenario.neighbors.iter()
            .map(|(pos, (_, light))| (*pos, light.clone()))
            .collect();

        block_group.bench_function(BenchmarkId::from_parameter(scenario.name), move |b| {
            b.iter(|| {
                let mut center_light = ChunkLight::default();
                let mut heightmap = ChunkHeightmap::default();
                let mut neighbor_lights: Vec<(IVec3, ChunkLight)> = neighbor_template.clone();
                let mut lights_map: HashMap<IVec3, &mut ChunkLight> = neighbor_lights.iter_mut()
                    .map(|(pos, light)| (*pos, light))
                    .collect();
                let mut dirty = 0;
                compute_light(
                    black_box(&scenario.center_chunk),
                    black_box(&mut center_light),
                    black_box(&mut heightmap),
                    black_box(&blocks),
                    black_box(&mut lights_map),
                    &mut dirty,
                    scenario.rendered,
                    0,
                    false,
                );
                light_on_place_block(
                    black_box(&scenario.center_chunk),
                    black_box(&mut center_light),
                    black_box(&blocks),
                    black_box(&mut lights_map),
                    black_box(scenario.place_pos),
                    &mut dirty,
                );
            });
        });
    }

    block_group.finish();
}

criterion_group!(benches, bench_full_light_compute, bench_incremental_light);
criterion_main!(benches);
