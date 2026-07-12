use std::{hint::black_box, sync::Arc};

use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::IVec3,
};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use minecraft_clone::{
    block::BlockType,
    world::{
        WorldMetadata,
        chunk::{
            CHUNK_SIZE, Chunk, ChunkCell, ChunkColumn, ChunkPos, LocalBlockPos,
            light::{ChunkHeightmap, ChunkLight, ChunkLightRegion, RebuiltChunkLight},
            mesh::ChunkMeshLight,
        },
        generation::generate_chunk,
    },
};

const SINGLE_TARGET_POSITION: ChunkPos = ChunkPos::new(-3, 0, 5);
const STAGED_PATCH_CENTER: ChunkColumn = ChunkColumn::new(-4, 7);

fn local(x: u32, y: u32, z: u32) -> LocalBlockPos {
    LocalBlockPos::new(x, y, z)
}

fn empty_chunk() -> Chunk {
    Chunk::default()
}

fn solid_chunk(block: BlockType) -> Chunk {
    Chunk::filled(block.into())
}

fn surface_terrain_chunk() -> Chunk {
    generate_chunk(&WorldMetadata::default(), SINGLE_TARGET_POSITION.as_ivec3())
}

fn checkerboard_leaves_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                if (x + y + z) % 2 == 0 {
                    chunk.set_cell_xyz(x, y, z, BlockType::OakLeaves.into());
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
                    chunk.set_cell_xyz(x, y, z, BlockType::Glowstone.into());
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
                let is_surface = x == 0
                    || x == CHUNK_SIZE - 1
                    || z == 0
                    || z == CHUNK_SIZE - 1
                    || y == 0
                    || y == CHUNK_SIZE - 1;
                if is_surface {
                    chunk.set_cell_xyz(x, y, z, BlockType::Stone.into());
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
                    chunk.set_cell_xyz(x, y, z, ChunkCell::EMPTY);
                }
                let wave2 = ((x as f32 * 1.3).cos() * 3.0 + 6.0) as i32;
                if (y as i32 - wave2).abs() < 2 && z > 2 && z < 13 && x > 1 && x < 14 {
                    chunk.set_cell_xyz(x, y, z, ChunkCell::EMPTY);
                }
            }
        }
    }
    chunk.set_cell_xyz(4, 8, 7, BlockType::Glowstone.into());
    chunk.set_cell_xyz(11, 8, 10, BlockType::Glowstone.into());
    chunk
}

fn glass_column_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            chunk.set_cell_xyz(x, 0, z, BlockType::Stone.into());
        }
    }
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 1..CHUNK_SIZE {
                if x % 3 == 0 && z % 3 == 0 {
                    let cell = if (y + x / 3 + z / 3) % 2 == 0 {
                        BlockType::Glass.into()
                    } else {
                        BlockType::OakLeaves.into()
                    };
                    chunk.set_cell_xyz(x, y, z, cell);
                }
            }
        }
    }
    chunk
}

fn face_boundary_lights(center: ChunkPos) -> HashMap<ChunkPos, ChunkLight> {
    let mut lights = HashMap::new();
    for offset in [
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Y,
        IVec3::NEG_Y,
        IVec3::Z,
        IVec3::NEG_Z,
    ] {
        let mut light = ChunkLight::default();
        for a in 0..CHUNK_SIZE as u32 {
            for b in 0..CHUNK_SIZE as u32 {
                let position = match offset {
                    IVec3::X => local(0, a, b),
                    IVec3::NEG_X => local(CHUNK_SIZE as u32 - 1, a, b),
                    IVec3::Y => local(a, 0, b),
                    IVec3::NEG_Y => local(a, CHUNK_SIZE as u32 - 1, b),
                    IVec3::Z => local(a, b, 0),
                    IVec3::NEG_Z => local(a, b, CHUNK_SIZE as u32 - 1),
                    _ => unreachable!(),
                };
                light.set_sky_light(position, 15);
                light.set_block_light(position, 15);
            }
        }
        lights.insert(center.offset(offset), light);
    }
    lights
}

struct SingleTargetScenario {
    name: &'static str,
    chunk: Chunk,
    boundary_lights: HashMap<ChunkPos, ChunkLight>,
}

impl SingleTargetScenario {
    fn isolated(name: &'static str, chunk: Chunk) -> Self {
        Self {
            name,
            chunk,
            boundary_lights: HashMap::new(),
        }
    }

    fn with_face_light(name: &'static str, chunk: Chunk) -> Self {
        Self {
            name,
            chunk,
            boundary_lights: face_boundary_lights(SINGLE_TARGET_POSITION),
        }
    }
}

fn single_target_scenarios() -> Vec<SingleTargetScenario> {
    vec![
        SingleTargetScenario::isolated("empty", empty_chunk()),
        SingleTargetScenario::isolated("solid_stone", solid_chunk(BlockType::Stone)),
        SingleTargetScenario::isolated("surface_terrain", surface_terrain_chunk()),
        SingleTargetScenario::isolated("checkerboard_leaves", checkerboard_leaves_chunk()),
        SingleTargetScenario::isolated("hollow_chamber", hollow_chamber_chunk()),
        SingleTargetScenario::isolated("cave", cave_chunk()),
        SingleTargetScenario::isolated("glass_column", glass_column_chunk()),
        SingleTargetScenario::isolated("glowstone_lattice", glowstone_lattice_chunk()),
        SingleTargetScenario::with_face_light("surface_terrain_face_lit", surface_terrain_chunk()),
    ]
}

fn bench_single_target_region_rebuild(c: &mut Criterion) {
    let scenarios = single_target_scenarios();
    let mut group = c.benchmark_group("light_single_target_region");
    group.throughput(Throughput::Elements(1));

    for scenario in &scenarios {
        let original_light = ChunkLight::default();
        let original_heightmap = ChunkHeightmap::default();
        group.bench_function(BenchmarkId::from_parameter(scenario.name), |b| {
            b.iter(|| {
                let mut region = ChunkLightRegion::new(1);
                region.insert_target(
                    SINGLE_TARGET_POSITION,
                    black_box(&scenario.chunk),
                    black_box(&original_light),
                    black_box(&original_heightmap),
                );
                for position in region.required_boundary_positions() {
                    if let Some(light) = scenario.boundary_lights.get(&position) {
                        region.insert_boundary_light(position, black_box(light));
                    }
                }
                black_box(region.rebuild());
            });
        });
    }

    group.finish();
}

struct MultiTargetScenario {
    name: &'static str,
    chunks: HashMap<ChunkPos, Chunk>,
    height_chunks: usize,
}

fn multi_target_scenarios() -> Vec<MultiTargetScenario> {
    let metadata = WorldMetadata::default();
    let height_chunks = metadata.height_chunks();
    let center = ChunkPos::new(-4, 0, 7);

    let mut empty_column = HashMap::new();
    let mut terrain_column = HashMap::new();
    for y in 0..height_chunks {
        let position = ChunkPos::new(center.as_ivec3().x, y as i32, center.as_ivec3().z);
        empty_column.insert(position, empty_chunk());
        terrain_column.insert(position, generate_chunk(&metadata, position.as_ivec3()));
    }

    let mut empty_columns = HashMap::new();
    let mut terrain_columns = HashMap::new();
    for x in -1..=1 {
        for z in -1..=1 {
            for y in 0..height_chunks {
                let position =
                    ChunkPos::new(center.as_ivec3().x + x, y as i32, center.as_ivec3().z + z);
                empty_columns.insert(position, empty_chunk());
                terrain_columns.insert(position, generate_chunk(&metadata, position.as_ivec3()));
            }
        }
    }

    vec![
        MultiTargetScenario {
            name: "empty_1_column",
            chunks: empty_column,
            height_chunks,
        },
        MultiTargetScenario {
            name: "terrain_1_column",
            chunks: terrain_column,
            height_chunks,
        },
        MultiTargetScenario {
            name: "empty_3x3_columns",
            chunks: empty_columns,
            height_chunks,
        },
        MultiTargetScenario {
            name: "terrain_3x3_columns",
            chunks: terrain_columns,
            height_chunks,
        },
    ]
}

fn bench_multi_target_region_rebuild(c: &mut Criterion) {
    let scenarios = multi_target_scenarios();
    let mut group = c.benchmark_group("light_multi_target_region");
    group.throughput(Throughput::Elements(1));

    for scenario in &scenarios {
        let lights = scenario
            .chunks
            .keys()
            .map(|&position| (position, ChunkLight::default()))
            .collect::<HashMap<_, _>>();
        let heightmaps = scenario
            .chunks
            .keys()
            .map(|&position| (position, ChunkHeightmap::default()))
            .collect::<HashMap<_, _>>();

        group.bench_function(BenchmarkId::from_parameter(scenario.name), |b| {
            b.iter(|| {
                let mut region = ChunkLightRegion::new(scenario.height_chunks);
                for (position, chunk) in &scenario.chunks {
                    region.insert_target(
                        *position,
                        black_box(chunk),
                        black_box(&lights[position]),
                        black_box(&heightmaps[position]),
                    );
                }
                black_box(region.rebuild());
            });
        });
    }

    group.finish();
}

#[derive(Debug)]
struct StagedPatchShape {
    calculation_columns: Vec<ChunkColumn>,
    commit_columns: Vec<ChunkColumn>,
}

impl StagedPatchShape {
    fn rectangular_core(min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> Self {
        Self {
            calculation_columns: columns_in_rectangle(min_x - 1, max_x + 1, min_z - 1, max_z + 1),
            commit_columns: columns_in_rectangle(min_x, max_x, min_z, max_z),
        }
    }

    fn calculation_chunk_count(&self, height_chunks: usize) -> usize {
        self.calculation_columns.len() * height_chunks
    }

    fn committed_chunk_count(&self, height_chunks: usize) -> usize {
        self.commit_columns.len() * height_chunks
    }
}

struct StagedPatchScenario {
    name: &'static str,
    height_chunks: usize,
    chunks: HashMap<ChunkPos, Chunk>,
    lights: HashMap<ChunkPos, ChunkLight>,
    heightmaps: HashMap<ChunkPos, ChunkHeightmap>,
    patches: Vec<StagedPatchShape>,
}

impl StagedPatchScenario {
    fn generated_terrain(name: &'static str, patches: Vec<StagedPatchShape>) -> Self {
        let metadata = WorldMetadata::default();
        let height_chunks = metadata.height_chunks();
        let mut calculation_columns = patches
            .iter()
            .flat_map(|patch| patch.calculation_columns.iter().copied())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        calculation_columns.sort_unstable_by_key(|column| (column.z(), column.x()));

        let positions = calculation_columns
            .into_iter()
            .flat_map(|column| (0..height_chunks as i32).map(move |y| column.chunk(y)))
            .collect::<Vec<_>>();
        let chunks = positions
            .iter()
            .copied()
            .map(|position| (position, generate_chunk(&metadata, position.as_ivec3())))
            .collect();
        let lights = positions
            .iter()
            .copied()
            .map(|position| (position, ChunkLight::default()))
            .collect();
        let heightmaps = positions
            .into_iter()
            .map(|position| (position, ChunkHeightmap::default()))
            .collect();

        Self {
            name,
            height_chunks,
            chunks,
            lights,
            heightmaps,
            patches,
        }
    }

    fn calculation_chunk_count(&self) -> usize {
        self.patches
            .iter()
            .map(|patch| patch.calculation_chunk_count(self.height_chunks))
            .sum()
    }

    fn committed_chunk_count(&self) -> usize {
        self.patches
            .iter()
            .map(|patch| patch.committed_chunk_count(self.height_chunks))
            .sum()
    }
}

fn columns_in_rectangle(min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> Vec<ChunkColumn> {
    (min_z..=max_z)
        .flat_map(|z| {
            (min_x..=max_x).map(move |x| {
                ChunkColumn::new(STAGED_PATCH_CENTER.x() + x, STAGED_PATCH_CENTER.z() + z)
            })
        })
        .collect()
}

fn staged_patch_scenarios() -> Vec<StagedPatchScenario> {
    let one_core = StagedPatchScenario::generated_terrain(
        "terrain_1_core_3x3_calculation",
        vec![StagedPatchShape::rectangular_core(0, 0, 0, 0)],
    );
    let four_cores = StagedPatchScenario::generated_terrain(
        "terrain_2x2_cores_4x4_calculation",
        vec![StagedPatchShape::rectangular_core(0, 1, 0, 1)],
    );
    let nine_cores_batched = StagedPatchScenario::generated_terrain(
        "terrain_3x3_cores_5x5_calculation",
        vec![StagedPatchShape::rectangular_core(-1, 1, -1, 1)],
    );
    let nine_cores_independent = StagedPatchScenario::generated_terrain(
        "terrain_9_independent_3x3_calculations",
        (-1..=1)
            .flat_map(|z| (-1..=1).map(move |x| StagedPatchShape::rectangular_core(x, x, z, z)))
            .collect(),
    );

    assert_eq!(one_core.calculation_chunk_count(), 45);
    assert_eq!(one_core.committed_chunk_count(), 5);
    assert_eq!(four_cores.calculation_chunk_count(), 80);
    assert_eq!(four_cores.committed_chunk_count(), 20);
    assert_eq!(nine_cores_batched.calculation_chunk_count(), 125);
    assert_eq!(nine_cores_batched.committed_chunk_count(), 45);
    assert_eq!(nine_cores_independent.calculation_chunk_count(), 405);
    assert_eq!(nine_cores_independent.committed_chunk_count(), 45);

    vec![
        one_core,
        four_cores,
        nine_cores_batched,
        nine_cores_independent,
    ]
}

fn solve_staged_patch(
    scenario: &StagedPatchScenario,
    patch: &StagedPatchShape,
) -> (Vec<(ChunkPos, Arc<[u32]>)>, Vec<RebuiltChunkLight>) {
    let mut region = ChunkLightRegion::new(scenario.height_chunks);
    for &column in &patch.calculation_columns {
        for y in 0..scenario.height_chunks as i32 {
            let position = column.chunk(y);
            region.insert_calculation_chunk(position, black_box(&scenario.chunks[&position]));
            if patch.commit_columns.contains(&column) {
                region.mark_commit_target(
                    position,
                    black_box(&scenario.lights[&position]),
                    black_box(&scenario.heightmaps[&position]),
                );
            }
        }
    }

    let solved = region.solve();
    let prepared_lights = {
        let solved_lights = solved.lights().collect::<HashMap<_, _>>();
        patch
            .commit_columns
            .iter()
            .flat_map(|column| (0..scenario.height_chunks as i32).map(move |y| column.chunk(y)))
            .map(|position| {
                let data = Arc::from(ChunkMeshLight::build_padded_data(position, &solved_lights));
                (position, data)
            })
            .collect()
    };
    (prepared_lights, solved.into_committed())
}

fn bench_staged_light_patches(c: &mut Criterion) {
    let scenarios = staged_patch_scenarios();
    let mut group = c.benchmark_group("light_staged_patch");

    for scenario in &scenarios {
        group.throughput(Throughput::Elements(scenario.committed_chunk_count() as u64));
        group.bench_function(BenchmarkId::from_parameter(scenario.name), |b| {
            b.iter(|| {
                let results = scenario
                    .patches
                    .iter()
                    .map(|patch| solve_staged_patch(black_box(scenario), patch))
                    .collect::<Vec<_>>();
                black_box(results)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_single_target_region_rebuild,
    bench_multi_target_region_rebuild,
    bench_staged_light_patches,
);
criterion_main!(benches);
