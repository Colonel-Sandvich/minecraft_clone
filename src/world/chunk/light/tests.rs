#[cfg(test)]
use super::*;

fn test_chunk_with_blocks<F>(mut fill: F) -> Chunk
where
    F: FnMut(u32, u32, u32) -> BlockType,
{
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                chunk.blocks[x][z][y] = fill(x as u32, y as u32, z as u32);
            }
        }
    }
    chunk
}

#[test]
fn sky_light_vertical_pass_above_surface_is_full() {
    let chunk = test_chunk_with_blocks(|_, y, _| {
        if y < 10 {
            BlockType::Stone
        } else {
            BlockType::Air
        }
    });
    let mut light = ChunkLight::default();
    let mut heightmap = ChunkHeightmap::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();

    let mut dirty = 0;
    compute_sky_light(
        &chunk,
        &mut light,
        &mut heightmap,
        &blocks,
        &mut lights,
        &mut dirty,
        0,
        false,
    );

    assert_eq!(heightmap.heights[8][8], 9);
    for y in 10..16 {
        assert_eq!(
            light.sky_light(uvec3(8, y, 8)),
            SKY_LIGHT_MAX,
            "sky light above surface at y={y}"
        );
    }
}

#[test]
fn sky_light_vertical_pass_attenuates_through_transparent() {
    let chunk = test_chunk_with_blocks(|x, y, z| {
        let _ = (x, z);
        if y < 10 {
            BlockType::Stone
        } else if y < 13 {
            BlockType::OakLeaves
        } else {
            BlockType::Air
        }
    });
    let mut light = ChunkLight::default();
    let mut heightmap = ChunkHeightmap::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();

    let mut dirty = 0;
    compute_sky_light(
        &chunk,
        &mut light,
        &mut heightmap,
        &blocks,
        &mut lights,
        &mut dirty,
        0,
        false,
    );

    assert_eq!(heightmap.heights[0][0], 9);
    assert_eq!(light.sky_light(uvec3(0, 15, 0)), SKY_LIGHT_MAX);
    assert_eq!(light.sky_light(uvec3(0, 14, 0)), SKY_LIGHT_MAX);
    assert_eq!(light.sky_light(uvec3(0, 12, 0)), SKY_LIGHT_MAX - 1);
    assert_eq!(light.sky_light(uvec3(0, 11, 0)), SKY_LIGHT_MAX - 2);
    assert_eq!(light.sky_light(uvec3(0, 9, 0)), 0);
}

#[test]
fn sky_light_vertical_pass_fully_opaque_stops_light() {
    let chunk = test_chunk_with_blocks(|_x, y, _z| {
        if y < 10 {
            BlockType::Stone
        } else {
            BlockType::Air
        }
    });
    let mut light = ChunkLight::default();
    let mut heightmap = ChunkHeightmap::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();

    let mut dirty = 0;
    compute_sky_light(
        &chunk,
        &mut light,
        &mut heightmap,
        &blocks,
        &mut lights,
        &mut dirty,
        0,
        false,
    );

    assert_eq!(heightmap.heights[0][0], 9);
    for y in 10..16 {
        assert_eq!(
            light.sky_light(uvec3(0, y as u32, 0)),
            15,
            "sky_light at y={y} should be 15"
        );
    }
    for y in 0..10 {
        let sl = light.sky_light(uvec3(0, y as u32, 0));
        assert_eq!(
            sl, 0,
            "sky_light at y={y} should be 0, but got {sl}. Block={:?}",
            chunk.blocks[0][0][y as usize]
        );
    }
}

#[test]
fn sky_light_horizontal_bfs_into_cave() {
    let chunk = test_chunk_with_blocks(|x, y, z| {
        if z == 8 && (x == 0 || (x == 1 && (6..=10).contains(&y))) {
            BlockType::Air
        } else {
            BlockType::Stone
        }
    });

    let mut light = ChunkLight::default();
    let mut heightmap = ChunkHeightmap::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();

    let mut dirty = 0;
    compute_sky_light(
        &chunk,
        &mut light,
        &mut heightmap,
        &blocks,
        &mut lights,
        &mut dirty,
        0,
        false,
    );

    assert_eq!(light.sky_light(uvec3(0, 15, 8)), SKY_LIGHT_MAX);
    assert!(light.sky_light(uvec3(1, 8, 8)) > 0);
    assert!(light.sky_light(uvec3(1, 8, 8)) < SKY_LIGHT_MAX);
}

#[test]
fn block_light_bfs_emitter_propagates() {
    let mut chunk = Chunk::default();
    chunk.blocks[8][8][8] = BlockType::Glowstone;

    let mut light = ChunkLight::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();

    let mut dirty = 0;
    compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);

    assert_eq!(light.block_light(uvec3(8, 8, 8)), 15);
    assert_eq!(light.block_light(uvec3(7, 8, 8)), 14);
    assert_eq!(light.block_light(uvec3(8, 9, 8)), 14);
    assert_eq!(light.block_light(uvec3(6, 8, 8)), 13);
}

#[test]
fn block_light_bfs_stopped_by_opaque() {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                chunk.blocks[x][z][y] = BlockType::Stone;
            }
        }
    }
    chunk.blocks[8][8][8] = BlockType::Glowstone;
    chunk.blocks[7][8][8] = BlockType::Air;

    let mut light = ChunkLight::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();

    let mut dirty = 0;
    compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);

    assert_eq!(light.block_light(uvec3(8, 8, 8)), 15);
    assert_eq!(light.block_light(uvec3(7, 8, 8)), 14);
    assert_eq!(light.block_light(uvec3(6, 8, 8)), 0);
}

#[test]
fn block_light_bfs_through_transparent() {
    let mut chunk = Chunk::default();
    chunk.blocks[8][8][8] = BlockType::Glowstone;
    chunk.blocks[7][8][8] = BlockType::OakLeaves;

    let mut light = ChunkLight::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();

    let mut dirty = 0;
    compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);

    assert_eq!(light.block_light(uvec3(8, 8, 8)), 15);
    assert_eq!(light.block_light(uvec3(7, 8, 8)), 14);
    assert_eq!(light.block_light(uvec3(6, 8, 8)), 13);
}

#[test]
fn cross_chunk_sky_light_propagates_upward() {
    let lower_chunk = Chunk::default();
    let mut lower_light = ChunkLight::default();
    let mut upper_chunk = Chunk::default();
    let mut upper_light = ChunkLight::default();
    let mut heightmap = ChunkHeightmap::default();

    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            upper_chunk.blocks[x][z][15] = BlockType::Air;
        }
    }

    lower_light.set_sky_light(uvec3(8, 15, 8), SKY_LIGHT_MAX);

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(0, -1, 0), &lower_chunk)]);
    let mut lights: HashMap<IVec3, ChunkLight> = HashMap::from([(ivec3(0, -1, 0), lower_light)]);
    let mut dirty = 0;
    compute_sky_light(
        &upper_chunk,
        &mut upper_light,
        &mut heightmap,
        &blocks,
        &mut lights,
        &mut dirty,
        0,
        false,
    );

    assert!(upper_light.sky_light(uvec3(8, 0, 8)) > 0);
}

#[test]
fn cross_chunk_block_light_propagates_between_chunks() {
    let left_chunk = Chunk::default();
    let left_light = ChunkLight::default();
    let mut right_chunk = Chunk::default();
    let mut right_light = ChunkLight::default();

    right_chunk.blocks[0][8][8] = BlockType::Glowstone;

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), left_light.clone())]);
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    let modified_left = lights.remove(&ivec3(-1, 0, 0)).unwrap();

    assert_eq!(right_light.block_light(uvec3(0, 8, 8)), 15);
    assert_eq!(right_light.block_light(uvec3(1, 8, 8)), 14);
    assert_eq!(modified_left.block_light(uvec3(15, 8, 8)), 14);
}

#[test]
fn block_light_decrease_when_emitter_removed() {
    let left_chunk = Chunk::default();
    let mut right_chunk = Chunk::default();
    let mut right_light = ChunkLight::default();

    right_chunk.blocks[0][8][8] = BlockType::Glowstone;

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

    // First: propagate light from Glowstone into neighbor.
    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), ChunkLight::default())]);
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    let left_after_emit = lights.remove(&ivec3(-1, 0, 0)).unwrap();
    assert_eq!(left_after_emit.block_light(uvec3(15, 8, 8)), 14);

    // Second: remove Glowstone and recompute.
    right_chunk.blocks[0][8][8] = BlockType::Air;
    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), left_after_emit)]);
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    clear_stale_neighbor_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    let left_after_remove = lights.remove(&ivec3(-1, 0, 0)).unwrap();

    assert_eq!(
        right_light.block_light(uvec3(0, 8, 8)),
        0,
        "emitter position should be 0"
    );
    assert_eq!(
        right_light.block_light(uvec3(1, 8, 8)),
        0,
        "no emitter means no center propagation"
    );
    assert_eq!(
        left_after_remove.block_light(uvec3(15, 8, 8)),
        0,
        "neighbor light must clear when emitter is removed"
    );
}

#[test]
fn light_packed_roundtrip() {
    let mut light = ChunkLight::default();
    let pos = uvec3(7, 5, 3);
    light.set_sky_light(pos, 13);
    light.set_block_light(pos, 9);

    assert_eq!(light.sky_light(pos), 13);
    assert_eq!(light.block_light(pos), 9);
    let packed = light.packed_light(pos);
    assert_eq!((packed >> 4) & 0x0F, 13);
    assert_eq!(packed & 0x0F, 9);
}

#[test]
fn padded_light_data_packs_four_cells_per_word() {
    let center_pos = IVec3::ZERO;
    let mut center = ChunkLight::default();
    center.set_sky_light(uvec3(0, 0, 0), 1);
    center.set_block_light(uvec3(0, 0, 0), 2);

    let mut right = ChunkLight::default();
    right.set_sky_light(uvec3(0, 0, 0), 10);
    right.set_block_light(uvec3(0, 0, 0), 11);

    let lights = HashMap::from([(center_pos, &center), (IVec3::X, &right)]);
    let data = ChunkLight::build_padded_light_data(center_pos, &lights);

    assert_eq!(data.len(), PADDED_LIGHT_WORDS);
    assert_eq!(
        unpack_padded_light(&data, padded_light_index(1, 1, 1)),
        0x12
    );
    assert_eq!(
        unpack_padded_light(&data, padded_light_index(17, 1, 1)),
        0xAB
    );
    assert_eq!(
        unpack_padded_light(&data, padded_light_index(0, 0, 0)),
        0xF0
    );
}

fn padded_light_index(x: usize, y: usize, z: usize) -> usize {
    x + z * PADDED_CHUNK_SIZE + y * PADDED_CHUNK_LAYER_SIZE
}

fn unpack_padded_light(data: &[u32], idx: usize) -> u8 {
    ((data[idx / 4] >> ((idx % 4) * 8)) & 0xFF) as u8
}

#[test]
fn light_resets_clear_correctly() {
    let mut light = ChunkLight::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                light.set_sky_light(uvec3(x as u32, y as u32, z as u32), 15);
                light.set_block_light(uvec3(x as u32, y as u32, z as u32), 15);
            }
        }
    }
    light.reset_all_sky_light();
    light.reset_all_block_light();

    assert_eq!(light.packed_light(uvec3(8, 8, 8)), 0);
}

#[test]
fn heightmap_all_air_chunk_is_zero() {
    let chunk = Chunk::default();
    let mut light = ChunkLight::default();
    let mut heightmap = ChunkHeightmap::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();

    let mut dirty = 0;
    compute_sky_light(
        &chunk,
        &mut light,
        &mut heightmap,
        &blocks,
        &mut lights,
        &mut dirty,
        0,
        false,
    );

    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            assert_eq!(heightmap.heights[x][z], 0);
        }
    }
}

fn empty_chunk() -> Chunk {
    Chunk::default()
}

fn target_set(positions: impl IntoIterator<Item = IVec3>) -> HashSet<IVec3> {
    positions.into_iter().collect()
}

#[test]
fn region_sky_occlusion_spans_vertical_chunks() {
    let lower = empty_chunk();
    let mut upper = empty_chunk();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            upper.blocks[x][z][0] = BlockType::Stone;
        }
    }

    let chunks: HashMap<IVec3, &Chunk> =
        HashMap::from([(ivec3(0, 0, 0), &lower), (ivec3(0, 1, 0), &upper)]);
    let mut lights = HashMap::from([
        (ivec3(0, 0, 0), ChunkLight::default()),
        (ivec3(0, 1, 0), ChunkLight::default()),
    ]);
    let mut heightmaps = HashMap::from([
        (ivec3(0, 0, 0), ChunkHeightmap::default()),
        (ivec3(0, 1, 0), ChunkHeightmap::default()),
    ]);
    let targets = target_set([ivec3(0, 0, 0), ivec3(0, 1, 0)]);

    compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 2);

    assert_eq!(lights[&ivec3(0, 1, 0)].sky_light(uvec3(8, 1, 8)), 15);
    assert_eq!(lights[&ivec3(0, 1, 0)].sky_light(uvec3(8, 0, 8)), 0);
    assert_eq!(lights[&ivec3(0, 0, 0)].sky_light(uvec3(8, 15, 8)), 0);
    assert_eq!(heightmaps[&ivec3(0, 0, 0)].heights[8][8], 16);
    assert_eq!(heightmaps[&ivec3(0, 1, 0)].heights[8][8], 16);
}

#[test]
fn region_sky_waits_for_missing_upper_chunk() {
    let lower = empty_chunk();
    let chunks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(0, 0, 0), &lower)]);
    let mut lights = HashMap::from([(ivec3(0, 0, 0), ChunkLight::default())]);
    let mut heightmaps = HashMap::from([(ivec3(0, 0, 0), ChunkHeightmap::default())]);
    let targets = target_set([ivec3(0, 0, 0)]);

    compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 2);

    assert_eq!(lights[&ivec3(0, 0, 0)].sky_light(uvec3(8, 15, 8)), 0);
}

#[test]
fn region_all_air_chunk_clears_stale_block_light() {
    let chunk = empty_chunk();
    let pos = IVec3::ZERO;
    let chunks: HashMap<IVec3, &Chunk> = HashMap::from([(pos, &chunk)]);
    let mut stale = ChunkLight::default();
    stale.set_block_light(uvec3(8, 8, 8), 15);
    let mut lights = HashMap::from([(pos, stale)]);
    let mut heightmaps = HashMap::from([(pos, ChunkHeightmap::default())]);
    let targets = target_set([pos]);

    compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 1);

    assert_eq!(lights[&pos].block_light(uvec3(8, 8, 8)), 0);
    assert_eq!(lights[&pos].sky_light(uvec3(8, 8, 8)), 15);
}

#[test]
fn region_block_light_crosses_y_boundary() {
    let lower = empty_chunk();
    let mut upper = empty_chunk();
    upper.blocks[8][8][0] = BlockType::Glowstone;
    let chunks: HashMap<IVec3, &Chunk> =
        HashMap::from([(ivec3(0, 0, 0), &lower), (ivec3(0, 1, 0), &upper)]);
    let mut lights = HashMap::from([
        (ivec3(0, 0, 0), ChunkLight::default()),
        (ivec3(0, 1, 0), ChunkLight::default()),
    ]);
    let mut heightmaps = HashMap::from([
        (ivec3(0, 0, 0), ChunkHeightmap::default()),
        (ivec3(0, 1, 0), ChunkHeightmap::default()),
    ]);
    let targets = target_set([ivec3(0, 0, 0), ivec3(0, 1, 0)]);

    compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 2);

    assert_eq!(lights[&ivec3(0, 1, 0)].block_light(uvec3(8, 0, 8)), 15);
    assert_eq!(lights[&ivec3(0, 0, 0)].block_light(uvec3(8, 15, 8)), 14);
}

#[test]
fn region_block_light_crosses_z_boundary() {
    let center = empty_chunk();
    let mut back = empty_chunk();
    back.blocks[8][0][8] = BlockType::Glowstone;
    let chunks: HashMap<IVec3, &Chunk> = HashMap::from([(IVec3::ZERO, &center), (IVec3::Z, &back)]);
    let mut lights = HashMap::from([
        (IVec3::ZERO, ChunkLight::default()),
        (IVec3::Z, ChunkLight::default()),
    ]);
    let mut heightmaps = HashMap::from([
        (IVec3::ZERO, ChunkHeightmap::default()),
        (IVec3::Z, ChunkHeightmap::default()),
    ]);
    let targets = target_set([IVec3::ZERO, IVec3::Z]);

    compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 1);

    assert_eq!(lights[&IVec3::Z].block_light(uvec3(8, 8, 0)), 15);
    assert_eq!(lights[&IVec3::ZERO].block_light(uvec3(8, 8, 15)), 14);
}

fn neighbor_with_glowstone(x: u32, y: u32, z: u32) -> (Chunk, ChunkLight) {
    let mut chunk = empty_chunk();
    chunk.blocks[x as usize][z as usize][y as usize] = BlockType::Glowstone;
    let mut light = ChunkLight::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();
    let mut dirty = 0;
    compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);
    (chunk, light)
}

// ── Pull-from-neighbor tests ──────────────────────────────────────────

#[test]
fn block_light_pulls_from_neighbor_emitter() {
    let left_chunk = empty_chunk();
    let mut left_light = ChunkLight::default();
    left_light.set_block_light(uvec3(15, 8, 8), 15);
    left_light.set_block_light(uvec3(14, 8, 8), 14);
    left_light.set_block_light(uvec3(15, 9, 8), 14);

    let right_chunk = empty_chunk();
    let mut right_light = ChunkLight::default();

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);
    let mut lights: HashMap<IVec3, ChunkLight> = HashMap::from([(ivec3(-1, 0, 0), left_light)]);
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    pull_neighbor_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );

    assert!(
        right_light.block_light(uvec3(0, 8, 8)) >= 14,
        "center must pull light from neighbor emitter; got {}",
        right_light.block_light(uvec3(0, 8, 8))
    );
    assert!(
        right_light.block_light(uvec3(1, 8, 8)) >= 13,
        "center propagation from pulled face light; got {}",
        right_light.block_light(uvec3(1, 8, 8))
    );
}

#[test]
fn block_light_pulls_from_neighbor_emitter_across_corner() {
    let diag_chunk = empty_chunk();
    let mut diag_light = ChunkLight::default();
    diag_light.set_block_light(uvec3(15, 8, 0), 15);
    diag_light.set_block_light(uvec3(14, 8, 0), 14);
    let center_chunk = empty_chunk();
    let mut center_light = ChunkLight::default();

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &diag_chunk)]);
    let mut lights: HashMap<IVec3, ChunkLight> = HashMap::from([(ivec3(-1, 0, 0), diag_light)]);
    let mut dirty = 0;
    compute_block_light(
        &center_chunk,
        &mut center_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    pull_neighbor_block_light(
        &center_chunk,
        &mut center_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );

    assert!(
        center_light.block_light(uvec3(0, 8, 0)) >= 14,
        "corner neighbor light must pull into center; got {}",
        center_light.block_light(uvec3(0, 8, 0))
    );
}

#[test]
fn empty_chunk_does_not_clear_neighbor_own_emitter_light() {
    let (left_chunk, left_light) = neighbor_with_glowstone(15, 8, 8);
    let right_chunk = empty_chunk();
    let mut right_light = ChunkLight::default();

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);
    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), left_light.clone())]);
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );

    let modified_left = lights.get(&ivec3(-1, 0, 0)).unwrap();
    assert_eq!(
        modified_left.block_light(uvec3(15, 8, 8)),
        15,
        "neighbor Glowstone must remain lit"
    );
    assert!(
        modified_left.block_light(uvec3(14, 8, 8)) > 0,
        "neighbor propagation from its own emitter must survive"
    );
}

#[test]
fn empty_chunk_neighbor_pull_from_multiple_sides() {
    let left_chunk = empty_chunk();
    let mut left_light = ChunkLight::default();
    left_light.set_block_light(uvec3(15, 8, 8), 15);
    left_light.set_block_light(uvec3(14, 8, 8), 14);

    let right_chunk = empty_chunk();
    let mut right_light = ChunkLight::default();
    right_light.set_block_light(uvec3(0, 8, 8), 15);
    right_light.set_block_light(uvec3(1, 8, 8), 14);

    let center_chunk = empty_chunk();
    let mut center_light = ChunkLight::default();

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([
        (ivec3(-1, 0, 0), &left_chunk),
        (ivec3(1, 0, 0), &right_chunk),
    ]);
    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), left_light), (ivec3(1, 0, 0), right_light)]);
    let mut dirty = 0;
    compute_block_light(
        &center_chunk,
        &mut center_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    pull_neighbor_block_light(
        &center_chunk,
        &mut center_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );

    assert!(
        center_light.block_light(uvec3(0, 8, 8)) >= 14,
        "light from left neighbor face; got {}",
        center_light.block_light(uvec3(0, 8, 8))
    );
    assert!(
        center_light.block_light(uvec3(15, 8, 8)) >= 14,
        "light from right neighbor face; got {}",
        center_light.block_light(uvec3(15, 8, 8))
    );
    let mid = center_light.block_light(uvec3(7, 8, 8));
    assert!(
        mid > 0,
        "midpoint should receive converging light; got {}",
        mid
    );
}

// ── Multiple emitters ─────────────────────────────────────────────────

#[test]
fn multiple_emitters_propagate_independently() {
    let mut chunk = empty_chunk();
    chunk.blocks[4][4][8] = BlockType::Glowstone;
    chunk.blocks[12][12][8] = BlockType::Glowstone;

    let mut light = ChunkLight::default();
    let blocks = HashMap::new();
    let mut lights = HashMap::new();
    let mut dirty = 0;
    compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);

    assert_eq!(light.block_light(uvec3(4, 8, 4)), 15);
    assert_eq!(light.block_light(uvec3(12, 8, 12)), 15);
    assert!(
        light.block_light(uvec3(7, 8, 7)) > 0,
        "midpoint between emitters must receive light"
    );
}

#[test]
fn multiple_emitters_on_faces_propagate_cross_chunk() {
    let left_chunk = empty_chunk();
    let mut right_chunk = empty_chunk();

    let left_light = ChunkLight::default();
    let mut right_light = ChunkLight::default();

    right_chunk.blocks[0][8][8] = BlockType::Glowstone;
    right_chunk.blocks[0][4][8] = BlockType::Glowstone;
    right_chunk.blocks[0][12][8] = BlockType::Glowstone;

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);
    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), left_light.clone())]);
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    let modified_left = lights.remove(&ivec3(-1, 0, 0)).unwrap();

    assert_eq!(modified_left.block_light(uvec3(15, 8, 8)), 14);
    assert_eq!(modified_left.block_light(uvec3(15, 8, 4)), 14);
    assert_eq!(modified_left.block_light(uvec3(15, 8, 12)), 14);
}

// ── Emitter removal with neighborhood ─────────────────────────────────

#[test]
fn removal_of_one_emitter_preserves_other_emitter_light() {
    let left_chunk = empty_chunk();
    let mut right_chunk = empty_chunk();
    right_chunk.blocks[0][8][8] = BlockType::Glowstone;
    right_chunk.blocks[0][8][10] = BlockType::Glowstone;

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), ChunkLight::default())]);
    let mut right_light = ChunkLight::default();
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );

    let left_after_both = lights.remove(&ivec3(-1, 0, 0)).unwrap();
    assert!(left_after_both.block_light(uvec3(15, 8, 8)) > 0);

    right_chunk.blocks[0][8][8] = BlockType::Air;
    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), left_after_both)]);
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    clear_stale_neighbor_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );

    let left_after_remove = lights.get(&ivec3(-1, 0, 0)).unwrap();
    assert_eq!(
        right_light.block_light(uvec3(0, 10, 8)),
        15,
        "remaining emitter must stay lit"
    );
    assert!(
        left_after_remove.block_light(uvec3(15, 10, 8)) > 0,
        "neighbor light from remaining emitter must survive; got 0"
    );
    assert!(
        left_after_remove.block_light(uvec3(15, 8, 8))
            < left_after_remove.block_light(uvec3(15, 10, 8)),
        "neighbor light at removed emitter location must decrease"
    );
}

#[test]
fn removal_of_all_emitters_clears_all_boundary_light() {
    let left_chunk = empty_chunk();
    let mut right_chunk = empty_chunk();
    right_chunk.blocks[0][8][8] = BlockType::Glowstone;

    let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

    let mut lights: HashMap<IVec3, ChunkLight> =
        HashMap::from([(ivec3(-1, 0, 0), ChunkLight::default())]);
    let mut right_light = ChunkLight::default();
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    let left_with = lights.remove(&ivec3(-1, 0, 0)).unwrap();
    assert_eq!(left_with.block_light(uvec3(15, 8, 8)), 14);

    right_chunk.blocks[0][8][8] = BlockType::Air;
    let mut lights: HashMap<IVec3, ChunkLight> = HashMap::from([(ivec3(-1, 0, 0), left_with)]);
    let mut dirty = 0;
    compute_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    clear_stale_neighbor_block_light(
        &right_chunk,
        &mut right_light,
        &blocks,
        &mut lights,
        &mut dirty,
    );
    let left_after = lights.remove(&ivec3(-1, 0, 0)).unwrap();

    for xz in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            assert_eq!(
                left_after.block_light(uvec3(xz as u32, y as u32, 8)),
                0,
                "all neighbor boundary light must be 0 after removal; got light at ({xz},{y},8)"
            );
        }
    }
}
