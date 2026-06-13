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
            Chunk,
            ambient_occlusion::AmbientOcclusionSettings,
            mesh::{ChunkMeshBlocks, ChunkMeshInput, ChunkMesher, GreedyChunkMesher},
        },
        generation::generate_chunk,
    },
};
use strum::IntoEnumIterator;

fn bench_chunk_mesh_pipeline(c: &mut Criterion) {
    let texture_map = bench_texture_map();
    let ao_brightness = AmbientOcclusionSettings::default().brightness_curve();

    let (all_positions, all_chunks) = generate_chunk_grid();

    let mut group = c.benchmark_group("chunk_mesh_pipeline");
    group.throughput(Throughput::Elements(1));

    for &dirty_count in &[1, 10, 50, 300] {
        let dirty_positions: Vec<IVec3> = all_positions
            .iter()
            .take(dirty_count.min(all_positions.len()))
            .copied()
            .collect();

        group.bench_function(
            BenchmarkId::from_parameter(format!("dirty_{dirty_count}")),
            |b| {
                b.iter(|| {
                    black_box(mesh_bulk(
                        black_box(&all_positions),
                        black_box(&all_chunks),
                        black_box(&dirty_positions),
                        black_box(&texture_map),
                        black_box(ao_brightness),
                    ))
                });
            },
        );
    }

    group.finish();
}

fn mesh_bulk(
    all_positions: &[IVec3],
    all_chunks: &[Chunk],
    dirty_positions: &[IVec3],
    block_texture_map: &BlockTextureMap,
    ao_brightness: [f32; 4],
) -> mesher_result::ChunkLayerMeshesCount {
    let chunks_by_pos: HashMap<IVec3, &Chunk> = all_positions
        .iter()
        .zip(all_chunks.iter())
        .map(|(p, c)| (*p, c))
        .collect();

    let mut total_faces = 0usize;

    for pos in dirty_positions {
        let blocks = ChunkMeshBlocks::from_chunks(*pos, &chunks_by_pos);
        let meshes = GreedyChunkMesher.mesh(ChunkMeshInput {
            blocks: &blocks,
            block_texture_map,
            ao_brightness,
        });
        for (_, mesh) in meshes {
            if let Some(indices) = mesh.indices() {
                total_faces += indices.len() / 3;
            }
        }
    }

    mesher_result::ChunkLayerMeshesCount {
        chunks: dirty_positions.len(),
        total_faces,
    }
}

mod mesher_result {
    pub struct ChunkLayerMeshesCount {
        #[allow(dead_code)]
        pub chunks: usize,
        #[allow(dead_code)]
        pub total_faces: usize,
    }
}

fn generate_chunk_grid() -> (Vec<IVec3>, Vec<Chunk>) {
    let metadata = WorldMetadata::default();
    let mut positions = Vec::with_capacity(324);
    let mut chunks = Vec::with_capacity(324);

    for y in 0..4i32 {
        for x in -4..5i32 {
            for z in -4..5i32 {
                let pos = ivec3(x, y, z);
                positions.push(pos);
                chunks.push(generate_chunk(&metadata, pos));
            }
        }
    }

    (positions, chunks)
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

criterion_group! {
    name = pipeline_benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3))
        .sample_size(10);
    targets = bench_chunk_mesh_pipeline
}
criterion_main!(pipeline_benches);
