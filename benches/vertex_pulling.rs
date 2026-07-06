//! Vertex-pulling mesh generation benchmark (8 scenarios).

use std::{hint::black_box, sync::Arc, time::Duration};

use bevy::{
    ecs::query::QueryState, platform::collections::HashMap, prelude::*, tasks::ComputeTaskPool,
    utils::Parallel,
};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use minecraft_clone::{
    block::{BlockMaterialLayer, BlockType},
    world::{
        WorldMetadata,
        chunk::mesh::{ChunkMeshBlocks, binary, vertex_pulling},
        chunk::{CHUNK_SIZE, Chunk, ChunkCell, ChunkLight, ChunkNeedsMeshRebuild, ChunkPosition},
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
    Chunk::filled(block.into())
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
    chunk.set_cell_xyz(8, 8, 8, BlockType::Stone.into());
    chunk
}

fn checkerboard_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                if (x + y + z) % 2 == 0 {
                    chunk.set_cell_xyz(x, y, z, BlockType::Stone.into());
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
                let cell = if y < 4 {
                    BlockType::Stone.into()
                } else if y < 6 {
                    BlockType::Dirt.into()
                } else if y == 6 {
                    BlockType::Grass.into()
                } else if (7..=11).contains(&y) && (6..=9).contains(&x) && (6..=9).contains(&z) {
                    BlockType::OakLog.into()
                } else if y == 12
                    && (5..=10).contains(&x)
                    && (5..=10).contains(&z)
                    && !((7..=8).contains(&x) && (7..=8).contains(&z))
                {
                    BlockType::OakLeaves.into()
                } else if y == 11 && (5..=10).contains(&x) && (5..=10).contains(&z) {
                    BlockType::OakLeaves.into()
                } else if y == 10
                    && (5..=10).contains(&x)
                    && (5..=10).contains(&z)
                    && (x == 5 || x == 10 || z == 5 || z == 10)
                {
                    BlockType::OakLeaves.into()
                } else if y == 3 && (x + z) % 13 == 0 {
                    BlockType::Glass.into()
                } else if y == 8 && (x * 7 + z * 11) % 23 == 0 {
                    BlockType::Glass.into()
                } else {
                    ChunkCell::EMPTY
                };
                chunk.set_cell_xyz(x, y, z, cell);
            }
        }
    }

    // Water pond on top (y=7), covering center 10×8 area, with ice on edges.
    for x in 3..=12 {
        for z in 3..=10 {
            let on_edge = x == 3 || x == 12 || z == 3 || z == 10;
            let cell = if on_edge {
                BlockType::Ice.into()
            } else {
                ChunkCell::water_source()
            };
            chunk.set_cell_xyz(x, 7, z, cell);
        }
    }
    // Second water layer (y=8) — smaller area, fills down into pond.
    for x in 5..=10 {
        for z in 6..=9 {
            if (x == 5 || x == 10 || z == 6 || z == 9)
                && chunk.cell_xyz(x, 7, z) == ChunkCell::EMPTY
            {
                continue;
            }
            chunk.set_cell_xyz(x, 8, z, ChunkCell::water_source());
        }
    }

    chunk
}

// ---------------------------------------------------------------------------
// Data size helpers
// ---------------------------------------------------------------------------

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
    ComputeTaskPool::get_or_init(Default::default);

    let scenarios = make_scenarios();

    bench_mesh_descriptors_scalar(c, &scenarios);
    bench_mesh_descriptors_hybrid(c, &scenarios);
    bench_mesh_descriptors_floor(c, &scenarios);
    print_data_size_comparison(&scenarios);
    print_bind_group_topology_comparison();
    bench_light_upload(c);
    bench_dirty_mesh_loop(c);
}

fn bench_mesh_descriptors_scalar(c: &mut Criterion, scenarios: &[Scenario]) {
    let mut group = c.benchmark_group("vp_mesh_scalar");
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

fn bench_mesh_descriptors_hybrid(c: &mut Criterion, scenarios: &[Scenario]) {
    let mut group = c.benchmark_group("vp_mesh_hybrid");
    group.throughput(Throughput::Elements(1));
    for scenario in scenarios {
        let chunk_refs = scenario.chunk_refs();
        let center = scenario.center_pos;
        group.bench_function(BenchmarkId::from_parameter(scenario.name), |b| {
            b.iter(|| {
                let blocks = ChunkMeshBlocks::from_chunks(center, black_box(&chunk_refs));
                black_box(binary::build_descriptors_hybrid(&blocks))
            });
        });
    }
    group.finish();
}

/// Absolute floor: masks + AND-NOT + iter_ones + coord recovery.
/// Same memory access pattern as `build_descriptors_binary` but skips AO,
/// cell lookup, and descriptor construction.
fn bench_mesh_descriptors_floor(c: &mut Criterion, scenarios: &[Scenario]) {
    let mut group = c.benchmark_group("vp_mesh_floor");
    group.throughput(Throughput::Elements(1));

    for scenario in scenarios {
        let chunk_refs = scenario.chunk_refs();
        let center = scenario.center_pos;
        let blocks = ChunkMeshBlocks::from_chunks(center, &chunk_refs);

        group.bench_function(BenchmarkId::from_parameter(scenario.name), move |b| {
            b.iter(|| binary::build_descriptors_binary_floor(black_box(&blocks)))
        });
    }
    group.finish();
}

fn print_data_size_comparison(scenarios: &[Scenario]) {
    println!();
    println!("--- Scalar vs Hybrid descriptor counts ---");
    println!("  {:<30} {:>8} {:>8}", "scenario", "scalar", "hybrid",);
    for scenario in scenarios {
        let chunk_refs = scenario.chunk_refs();
        let blocks = ChunkMeshBlocks::from_chunks(scenario.center_pos, &chunk_refs);
        let scalar_faces = vp_face_count(&vertex_pulling::build_descriptors(&blocks));
        let hybrid_faces = vp_face_count(&binary::build_descriptors_hybrid(&blocks));

        println!(
            "  {:<30} {:>8} {:>8}",
            scenario.name, scalar_faces, hybrid_faces,
        );
    }
    println!();
}

#[derive(Clone, Copy, Debug)]
struct BindGroupWork {
    mesh_update_groups: usize,
    light_update_groups: usize,
    light_update_writes: usize,
    draw_set_calls: usize,
}

fn total_layers(layer_counts: &[usize]) -> usize {
    layer_counts.iter().sum()
}

fn non_empty_chunks(layer_counts: &[usize]) -> usize {
    layer_counts.iter().filter(|&&layers| layers > 0).count()
}

fn realistic_layer_counts(chunk_count: usize) -> Vec<usize> {
    let chunk = realistic_terrain_chunk();
    let mut chunk_refs = HashMap::default();
    chunk_refs.insert(IVec3::ZERO, &chunk);
    let blocks = ChunkMeshBlocks::from_chunks(IVec3::ZERO, &chunk_refs);
    let layers = binary::build_descriptors_hybrid(&blocks).len();

    vec![layers; chunk_count]
}

fn combined_group_work(layer_counts: &[usize]) -> BindGroupWork {
    let layers = total_layers(layer_counts);
    let chunks = non_empty_chunks(layer_counts);
    BindGroupWork {
        mesh_update_groups: layers,
        light_update_groups: 0,
        light_update_writes: chunks,
        draw_set_calls: layers * 2,
    }
}

fn split_group_work(layer_counts: &[usize]) -> BindGroupWork {
    let layers = total_layers(layer_counts);
    let chunks = non_empty_chunks(layer_counts);
    BindGroupWork {
        mesh_update_groups: layers + chunks,
        light_update_groups: chunks,
        light_update_writes: 0,
        draw_set_calls: layers * 3,
    }
}

fn print_bind_group_topology_comparison() {
    println!("--- Vertex-pulling bind group topology (combined/write vs split/recreate) ---");
    println!(
        "  {:>6} {:>7} {:>18} {:>18} {:>20} {:>20} {:>18} {:>15}",
        "chunks",
        "layers",
        "combined mesh BGs",
        "split mesh BGs",
        "combined light BG/wr",
        "split light BG/wr",
        "combined draw sets",
        "split draw sets",
    );
    for chunk_count in [64usize, 512, 4096] {
        let layer_counts = realistic_layer_counts(chunk_count);
        let layers = total_layers(&layer_counts);
        let combined = combined_group_work(&layer_counts);
        let split = split_group_work(&layer_counts);
        println!(
            "  {chunk_count:>6} {layers:>7} {combined_mesh:>18} {split_mesh:>18} {combined_light_bg:>8}/{combined_writes:<8} {split_light_bg:>8}/{split_writes:<8} {combined_draws:>18} {split_draws:>15}",
            combined_mesh = combined.mesh_update_groups,
            split_mesh = split.mesh_update_groups,
            combined_light_bg = combined.light_update_groups,
            combined_writes = combined.light_update_writes,
            split_light_bg = split.light_update_groups,
            split_writes = split.light_update_writes,
            combined_draws = combined.draw_set_calls,
            split_draws = split.draw_set_calls,
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

fn dirty_mesh_chunks(chunk_count: usize) -> Vec<(IVec3, Chunk)> {
    let chunk = realistic_terrain_chunk();
    let edge = (chunk_count as f32).cbrt().ceil() as i32;
    let mut chunks = Vec::with_capacity(chunk_count);
    for x in 0..edge {
        for y in 0..edge {
            for z in 0..edge {
                if chunks.len() == chunk_count {
                    return chunks;
                }
                chunks.push((ivec3(x, y, z), chunk.clone()));
            }
        }
    }
    chunks
}

fn dirty_mesh_world(chunks: &[(IVec3, Chunk)]) -> World {
    let mut world = World::new();
    for (pos, chunk) in chunks {
        world.spawn((ChunkPosition(*pos), chunk.clone(), ChunkNeedsMeshRebuild));
    }
    world
}

fn build_dirty_meshes_serial_contiguous(
    world: &World,
    query: &mut QueryState<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    chunks_by_pos: &HashMap<IVec3, &Chunk>,
) -> usize {
    let mut face_count = 0usize;
    for (positions, _) in query
        .contiguous_iter(world)
        .expect("dirty mesh query should stay dense")
    {
        for pos in positions {
            let blocks = ChunkMeshBlocks::from_chunks(pos.0, chunks_by_pos);
            face_count += vp_face_count(&binary::build_descriptors_hybrid(&blocks));
        }
    }
    face_count
}

fn build_dirty_meshes_parallel(
    world: &World,
    query: &mut QueryState<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    chunks_by_pos: &HashMap<IVec3, &Chunk>,
) -> usize {
    let mut totals = Parallel::<usize>::default();
    query.par_iter(world).for_each_init(
        || totals.borrow_local_mut(),
        |local_total, (pos, _)| {
            let blocks = ChunkMeshBlocks::from_chunks(pos.0, chunks_by_pos);
            **local_total += vp_face_count(&binary::build_descriptors_hybrid(&blocks));
        },
    );

    totals.iter_mut().map(|total| *total).sum()
}

fn bench_dirty_mesh_loop(c: &mut Criterion) {
    let mut group = c.benchmark_group("vp_dirty_mesh_loop");
    for chunk_count in [1usize, 4, 16, 64, 256] {
        let chunks = dirty_mesh_chunks(chunk_count);
        let chunks_by_pos = chunks
            .iter()
            .map(|(pos, chunk)| (*pos, chunk))
            .collect::<HashMap<_, _>>();
        let mut world = dirty_mesh_world(&chunks);

        group.throughput(Throughput::Elements(chunk_count as u64));
        let mut query = world
            .query_filtered::<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>(
            );
        group.bench_function(BenchmarkId::new("serial_contiguous", chunk_count), |b| {
            b.iter(|| {
                black_box(build_dirty_meshes_serial_contiguous(
                    &world,
                    &mut query,
                    black_box(&chunks_by_pos),
                ))
            })
        });

        let mut query = world
            .query_filtered::<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>(
            );
        group.bench_function(BenchmarkId::new("parallel", chunk_count), |b| {
            b.iter(|| {
                black_box(build_dirty_meshes_parallel(
                    &world,
                    &mut query,
                    black_box(&chunks_by_pos),
                ))
            })
        });
    }
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
