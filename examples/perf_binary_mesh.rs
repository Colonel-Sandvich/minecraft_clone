//! Perf-friendly binary mesh microbenchmark.
//! Compile:  cargo build --release --example perf_binary_mesh
//! Profile:  perf stat -e task-clock,cycles,instructions,cache-references,cache-misses,branches,branch-misses \
//!               target/release/examples/perf_binary_mesh
//!
//! Flamegraph:
//!   perf record -g target/release/examples/perf_binary_mesh
//!   perf report --stdio -g graph --no-children --symbol-filter=binary   # or your pkg name

use bevy::{platform::collections::HashMap, prelude::IVec3};
use minecraft_clone::{
    item::Item,
    world::chunk::{
        CHUNK_SIZE, Chunk, ChunkCell,
        mesh::{ChunkMeshBlocks, mesher::build_binary},
    },
};
use std::hint::black_box;

fn main() {
    let chunk = realistic_terrain_chunk();
    let mut chunk_refs: HashMap<IVec3, &Chunk> = HashMap::default();
    chunk_refs.insert(IVec3::ZERO, &chunk);

    let blocks = ChunkMeshBlocks::from_chunks(IVec3::ZERO, &chunk_refs);

    // Prime the cache
    for _ in 0..10_000 {
        black_box(build_binary(&blocks));
    }

    // Timed loop
    let iterations = 200_000;
    let start = std::time::Instant::now();
    for _ in 0..iterations {
        black_box(build_binary(&blocks));
    }
    let elapsed = start.elapsed();

    let per_iter = elapsed / iterations;
    println!(
        "binary mesh: {:?}/iter  ({} iters / {:?})",
        per_iter, iterations, elapsed
    );
}

fn realistic_terrain_chunk() -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                let cell = if y < 4 {
                    Item::Stone.into()
                } else if y < 6 {
                    Item::Dirt.into()
                } else if y == 6 {
                    Item::Grass.into()
                } else if (7..=11).contains(&y) && (6..=9).contains(&x) && (6..=9).contains(&z) {
                    Item::OakLog.into()
                } else if y == 12
                    && (5..=10).contains(&x)
                    && (5..=10).contains(&z)
                    && !((7..=8).contains(&x) && (7..=8).contains(&z))
                {
                    Item::OakLeaves.into()
                } else if y == 11 && (5..=10).contains(&x) && (5..=10).contains(&z) {
                    Item::OakLeaves.into()
                } else if y == 10
                    && (5..=10).contains(&x)
                    && (5..=10).contains(&z)
                    && (x == 5 || x == 10 || z == 5 || z == 10)
                {
                    Item::OakLeaves.into()
                } else if y == 3 && (x + z) % 13 == 0 {
                    Item::Glass.into()
                } else if y == 8 && (x * 7 + z * 11) % 23 == 0 {
                    Item::Glass.into()
                } else {
                    ChunkCell::EMPTY
                };
                chunk.set_cell_xyz(x, y, z, cell);
            }
        }
    }

    // Water pond
    for x in 3..=12 {
        for z in 3..=10 {
            let on_edge = x == 3 || x == 12 || z == 3 || z == 10;
            let cell = if on_edge {
                Item::Ice.into()
            } else {
                ChunkCell::water_source()
            };
            chunk.set_cell_xyz(x, 7, z, cell);
        }
    }
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
