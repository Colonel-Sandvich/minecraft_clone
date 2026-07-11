use super::*;
use crate::block::{
    BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED, BlockType, WATER_RENDER_ID, render_id_for_block,
};
use crate::world::chunk::CHUNK_SIZE;
use crate::world::chunk::mesh::{
    ChunkMeshBlocks,
    blocks::{PADDED_CHUNK_VOLUME, padded_chunk_index},
    mesher::visibility::block_mesh_flags,
};
use crate::world::chunk::{Chunk, ChunkCell};

fn make_padded(kinds: &[u16]) -> ChunkMeshBlocks {
    let mut blocks = Box::new([0u16; PADDED_CHUNK_VOLUME]);
    let mut fluid_levels = Box::new([0u8; PADDED_CHUNK_VOLUME]);
    blocks[..kinds.len()].copy_from_slice(kinds);
    for (i, &kind) in kinds.iter().enumerate() {
        if kind == WATER_RENDER_ID {
            fluid_levels[i] = 8;
        }
    }
    let mut center_rendered = 0u16;
    let mut center_full = 0u16;
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let idx = padded_chunk_index(x + 1, y + 1, z + 1);
                let flags = block_mesh_flags(blocks[idx]);
                if flags & BLOCK_FLAG_RENDERED != 0 {
                    center_rendered += 1;
                }
                if flags & BLOCK_FLAG_FULL_CUBE != 0 {
                    center_full += 1;
                }
            }
        }
    }
    ChunkMeshBlocks {
        blocks,
        fluid_levels,
        center_rendered_blocks: center_rendered,
        center_full_cube_blocks: center_full,
        neighbor_face_shells_full_cube: false,
    }
}

fn face_keys(layers: Vec<LayerMesh>) -> Vec<(usize, u32, u32)> {
    let mut keys = layers
        .into_iter()
        .flat_map(|layer| {
            layer.faces.into_iter().map(move |face| {
                let [packed, info] = face.words();
                (layer.material_layer.index(), packed, info)
            })
        })
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

fn assert_binary_faces_match_scalar(blocks: &ChunkMeshBlocks, label: &str) {
    let scalar = face_keys(super::super::build_reference(blocks));
    let binary = face_keys(build_binary(blocks));

    assert_eq!(binary, scalar, "{label}");
}

#[test]
fn plane_bits_set_and_iterate() {
    let mut pb = PlaneBits::zeroed();
    pb.set(0);
    pb.set(5);
    pb.set(63);
    pb.set(64);
    pb.set(323);
    let ones: Vec<_> = pb.iter_ones().collect();
    assert_eq!(ones, vec![0, 5, 63, 64, 323]);
}

#[test]
fn and_not_clears_matching_bits() {
    let mut a = PlaneBits::zeroed();
    let mut b = PlaneBits::zeroed();
    a.set(10);
    a.set(20);
    b.set(10);
    b.set(30);
    let result = a.and_not(&b);
    let ones: Vec<_> = result.iter_ones().collect();
    assert_eq!(ones, vec![20]);
}

#[test]
fn binary_empty_chunk() {
    let padded = make_padded(&[0u16; PADDED_CHUNK_VOLUME]);
    let result = build_binary(&padded);
    assert!(result.is_empty());
}

#[test]
fn binary_single_full_cube_emits_six_faces() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    let center = padded_chunk_index(9, 9, 9);
    kinds[center] = render_id_for_block(BlockType::Stone);
    let padded = make_padded(&kinds);
    let result = build_binary(&padded);
    let total: usize = result.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(total, 6, "single stone should emit 6 faces");
}

#[test]
fn binary_full_cube_ao_faces_match_scalar() {
    let stone = render_id_for_block(BlockType::Stone);
    let target = padded_chunk_index(8, 8, 8);

    for side in 0..DIRECTION_COUNT {
        for sample in 0..FACE_AO_SAMPLE_COUNT {
            let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
            kinds[target] = stone;
            kinds[(target as isize + FACE_AO_SAMPLE_OFFSETS[side][sample]) as usize] = stone;

            let padded = make_padded(&kinds);
            assert_binary_faces_match_scalar(&padded, &format!("side={side} sample={sample}"));
        }
    }
}

#[test]
fn binary_two_adjacent_full_cubes_emit_ten_faces() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    let a = padded_chunk_index(9, 9, 9); // center
    let b = padded_chunk_index(10, 9, 9); // +X neighbor
    kinds[a] = render_id_for_block(BlockType::Stone);
    kinds[b] = render_id_for_block(BlockType::Stone);
    let padded = make_padded(&kinds);
    let result = build_binary(&padded);
    let total: usize = result.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(total, 10, "two adjacent stones share 2 faces → 10 faces");
}

#[test]
fn binary_full_cube_buried_emits_nothing() {
    let kinds = [render_id_for_block(BlockType::Stone); PADDED_CHUNK_VOLUME];
    let padded = make_padded(&kinds);
    let result = build_binary(&padded);
    let total: usize = result.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(total, 0, "fully buried full-cube chunk emits 0 faces");
}

#[test]
fn binary_stone_next_to_glass_emits_six_faces() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    let stone = padded_chunk_index(9, 9, 9);
    let glass = padded_chunk_index(10, 9, 9); // +X neighbor
    kinds[stone] = render_id_for_block(BlockType::Stone);
    kinds[glass] = render_id_for_block(BlockType::Glass); // Glass is rendered but NOT full_cube
    let padded = make_padded(&kinds);
    let result = build_binary(&padded);
    let total: usize = result.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(
        total, 6,
        "binary path only handles full-cube; stone emits 6 faces"
    );
}

#[test]
fn hybrid_stone_plus_glass_matches_scalar() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    let stone = padded_chunk_index(9, 9, 9);
    let glass = padded_chunk_index(10, 9, 9); // +X neighbor
    kinds[stone] = render_id_for_block(BlockType::Stone);
    kinds[glass] = render_id_for_block(BlockType::Glass);
    let padded = make_padded(&kinds);

    let scalar = super::super::build_reference(&padded);
    let hybrid = super::super::build(&padded);

    let scalar_total: usize = scalar.iter().map(|layer| layer.faces.len()).sum();
    let hybrid_total: usize = hybrid.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(
        hybrid_total, scalar_total,
        "hybrid must match scalar face count"
    );
}

#[test]
fn hybrid_water_stone_matches_scalar() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    let water = padded_chunk_index(9, 9, 9);
    let stone = padded_chunk_index(10, 9, 9);
    kinds[water] = WATER_RENDER_ID;
    kinds[stone] = render_id_for_block(BlockType::Stone);
    let padded = make_padded(&kinds);

    let scalar = super::super::build_reference(&padded);
    let hybrid = super::super::build(&padded);

    let scalar_total: usize = scalar.iter().map(|layer| layer.faces.len()).sum();
    let hybrid_total: usize = hybrid.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(
        hybrid_total, scalar_total,
        "hybrid water+stone must match scalar"
    );
}

#[test]
fn hybrid_dense_non_full_cube_matches_scalar() {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            chunk.set_cell_xyz(x, 7, z, ChunkCell::water_source());
        }
    }

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    assert!(
        blocks.has_non_full_cube_rendered(),
        "water sheet should use the non-full-cube path"
    );

    let scalar = face_keys(super::super::build_reference(&blocks));
    let hybrid = face_keys(super::super::build(&blocks));
    assert_eq!(hybrid, scalar);
}

// This function compares hybrid output against scalar for correctness.
// Individual sub-scenarios are tested inline with separate assert_eq calls
// so failure messages identify the failing sub-scenario.
#[test]
fn hybrid_full_scenarios_match_scalar() {
    // empty
    {
        let padded = make_padded(&[0u16; PADDED_CHUNK_VOLUME]);
        let scalar = super::super::build_reference(&padded);
        let hybrid = super::super::build(&padded);
        assert_eq!(
            scalar.iter().map(|layer| layer.faces.len()).sum::<usize>(),
            hybrid.iter().map(|layer| layer.faces.len()).sum::<usize>(),
            "empty"
        );
    }
    // all stone
    {
        let padded = make_padded(&[render_id_for_block(BlockType::Stone); PADDED_CHUNK_VOLUME]);
        let scalar = super::super::build_reference(&padded);
        let hybrid = super::super::build(&padded);
        assert_eq!(
            scalar.iter().map(|layer| layer.faces.len()).sum::<usize>(),
            hybrid.iter().map(|layer| layer.faces.len()).sum::<usize>(),
            "all stone buried"
        );
    }
    // checkerboard
    {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    if (x + y + z) % 2 == 0 {
                        kinds[padded_chunk_index(x + 1, y + 1, z + 1)] =
                            render_id_for_block(BlockType::Stone);
                    }
                }
            }
        }
        let padded = make_padded(&kinds);
        let scalar = super::super::build_reference(&padded);
        let hybrid = super::super::build(&padded);
        assert_eq!(
            scalar.iter().map(|layer| layer.faces.len()).sum::<usize>(),
            hybrid.iter().map(|layer| layer.faces.len()).sum::<usize>(),
            "checkerboard"
        );
    }
    // stone + glass mixed
    {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        kinds[padded_chunk_index(9, 9, 9)] = render_id_for_block(BlockType::Stone);
        kinds[padded_chunk_index(9, 9, 8)] = render_id_for_block(BlockType::Stone);
        kinds[padded_chunk_index(9, 9, 10)] = render_id_for_block(BlockType::Glass);
        kinds[padded_chunk_index(9, 8, 9)] = render_id_for_block(BlockType::OakLeaves);
        kinds[padded_chunk_index(9, 10, 9)] = WATER_RENDER_ID;
        let padded = make_padded(&kinds);
        let scalar = super::super::build_reference(&padded);
        let hybrid = super::super::build(&padded);
        assert_eq!(
            scalar.iter().map(|layer| layer.faces.len()).sum::<usize>(),
            hybrid.iter().map(|layer| layer.faces.len()).sum::<usize>(),
            "mixed block types"
        );
    }
}

#[test]
fn translucent_water_culled_by_stone() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    let water = padded_chunk_index(9, 9, 9);
    let stone = padded_chunk_index(10, 9, 9);
    kinds[water] = WATER_RENDER_ID;
    kinds[stone] = render_id_for_block(BlockType::Stone);
    let padded = make_padded(&kinds);

    let result = super::super::build_reference(&padded);
    let total: usize = result.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(total, 11, "water+stone: 6 stone + 5 water faces");
}

#[test]
fn translucent_water_basin() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    kinds[padded_chunk_index(9, 9, 9)] = WATER_RENDER_ID;
    kinds[padded_chunk_index(9, 8, 9)] = render_id_for_block(BlockType::Stone); // below
    kinds[padded_chunk_index(10, 9, 9)] = render_id_for_block(BlockType::Stone); // +X
    kinds[padded_chunk_index(8, 9, 9)] = render_id_for_block(BlockType::Stone); // -X
    kinds[padded_chunk_index(9, 9, 10)] = render_id_for_block(BlockType::Stone); // +Z
    kinds[padded_chunk_index(9, 9, 8)] = render_id_for_block(BlockType::Stone); // -Z
    let padded = make_padded(&kinds);

    let result = super::super::build_reference(&padded);
    let total: usize = result.iter().map(|layer| layer.faces.len()).sum();
    // Water: 1 up face. Five stone blocks: 6 faces each (water doesn't occlude).
    // Total = 1 + 6*5 = 31
    assert_eq!(total, 31, "water basin: 1 water up + 5*6 stone faces");
}

#[test]
fn translucent_ice_culled_by_stone() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    kinds[padded_chunk_index(9, 9, 9)] = render_id_for_block(BlockType::Ice);
    kinds[padded_chunk_index(9, 8, 9)] = render_id_for_block(BlockType::Stone);
    let padded = make_padded(&kinds);

    let result = super::super::build_reference(&padded);
    let total: usize = result.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(total, 11, "ice+stone: 6 stone + 5 ice faces");
}

#[test]
fn translucent_water_adjacent_water_culled() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    kinds[padded_chunk_index(9, 9, 9)] = WATER_RENDER_ID;
    kinds[padded_chunk_index(9, 9, 10)] = WATER_RENDER_ID;
    let padded = make_padded(&kinds);

    let result = super::super::build_reference(&padded);
    let total: usize = result.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(total, 10, "adjacent water: 10 faces (2 culled)");
}

#[test]
fn water_connectivity_different_levels() {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    let idx_a = padded_chunk_index(9, 9, 9);
    let idx_b = padded_chunk_index(10, 9, 9);
    kinds[idx_a] = WATER_RENDER_ID;
    kinds[idx_b] = WATER_RENDER_ID;
    let mut padded = make_padded(&kinds);
    padded.fluid_levels[idx_a] = 5;
    padded.fluid_levels[idx_b] = 3;

    let scalar = super::super::build_reference(&padded);
    let hybrid = super::super::build(&padded);
    let scalar_total: usize = scalar.iter().map(|layer| layer.faces.len()).sum();
    let hybrid_total: usize = hybrid.iter().map(|layer| layer.faces.len()).sum();
    assert_eq!(
        hybrid_total, scalar_total,
        "different water levels: hybrid matches scalar"
    );
    // A (level 5): 5 faces (water-water side culled, top slopes via corner heights)
    // B (level 3): 5 faces (same)
    // Total = 10
    assert_eq!(
        scalar_total, 10,
        "different water levels: 10 faces (top slopes)"
    );
}

// Recreate the realistic terrain used in the benchmark.
// Inline `make_padded` call to avoid depending on tests-only helper.
#[allow(clippy::if_same_then_else, clippy::manual_range_contains)]
fn make_realistic_padded() -> ChunkMeshBlocks {
    let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                let idx = padded_chunk_index(x + 1, y + 1, z + 1);
                kinds[idx] = if y < 4 {
                    render_id_for_block(BlockType::Stone)
                } else if y < 6 {
                    render_id_for_block(BlockType::Dirt)
                } else if y == 6 {
                    render_id_for_block(BlockType::Grass)
                } else if y >= 7 && y <= 11 && x >= 6 && x <= 9 && z >= 6 && z <= 9 {
                    render_id_for_block(BlockType::OakLog)
                } else if y == 12
                    && x >= 5
                    && x <= 10
                    && z >= 5
                    && z <= 10
                    && !(x >= 7 && x <= 8 && z >= 7 && z <= 8)
                {
                    render_id_for_block(BlockType::OakLeaves)
                } else if y == 11 && x >= 5 && x <= 10 && z >= 5 && z <= 10 {
                    render_id_for_block(BlockType::OakLeaves)
                } else if y == 10
                    && x >= 5
                    && x <= 10
                    && z >= 5
                    && z <= 10
                    && (x == 5 || x == 10 || z == 5 || z == 10)
                {
                    render_id_for_block(BlockType::OakLeaves)
                } else if y == 3 && (x + z) % 13 == 0 {
                    render_id_for_block(BlockType::Glass)
                } else if y == 8 && (x * 7 + z * 11) % 23 == 0 {
                    render_id_for_block(BlockType::Glass)
                } else {
                    0u16
                };
            }
        }
    }
    make_padded(&kinds)
}

#[test]
fn perf_breakdown_binary() {
    use std::time::Instant;

    let blocks = make_realistic_padded();
    const ITERS: u64 = 50_000;

    // measure mask construction
    let t0 = Instant::now();
    for _ in 0..ITERS {
        let masks = BinaryFaceMasks::from_padded(&blocks);
        std::hint::black_box(masks);
    }
    let mask_elapsed = t0.elapsed();

    // measure full binary meshing
    let t0 = Instant::now();
    for _ in 0..ITERS {
        let result = build_binary(&blocks);
        std::hint::black_box(result);
    }
    let full_elapsed = t0.elapsed();

    let mask_ns = mask_elapsed.as_nanos() as f64 / ITERS as f64;
    let full_ns = full_elapsed.as_nanos() as f64 / ITERS as f64;
    let rest_ns = full_ns - mask_ns;

    let face_count = build_binary(&blocks)
        .iter()
        .map(|layer| layer.faces.len())
        .sum::<usize>();

    println!();
    println!(
        "=== binary mesh breakdown (realistic, {} faces) ===",
        face_count
    );
    println!(
        "  mask build:           {:>9.0} ns  ({:.1}%)",
        mask_ns,
        mask_ns / full_ns * 100.0
    );
    println!(
        "  cull+emit+collect:    {:>9.0} ns  ({:.1}%)",
        rest_ns,
        rest_ns / full_ns * 100.0
    );
    println!("  total:                {:>9.0} ns", full_ns);
    println!(
        "  per-face:             {:>9.0} ns",
        full_ns / face_count as f64
    );
    println!();
}
