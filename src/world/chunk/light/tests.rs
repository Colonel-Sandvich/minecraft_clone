use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
};

use crate::{
    block::BlockType,
    world::chunk::{CHUNK_SIZE, Chunk, ChunkCell, ChunkPos, LocalBlockPos},
};

use super::storage::SKY_LIGHT_MAX;
use super::{ChunkHeightmap, ChunkLight, ChunkLightRegion, RebuiltChunkLight};

fn local(x: u32, y: u32, z: u32) -> LocalBlockPos {
    LocalBlockPos::new(x, y, z)
}

fn block_cell(block: BlockType) -> ChunkCell {
    block.into()
}

fn chunk_with_cells(mut cell_at: impl FnMut(u32, u32, u32) -> ChunkCell) -> Chunk {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                chunk.set_cell_xyz(x, y, z, cell_at(x as u32, y as u32, z as u32));
            }
        }
    }
    chunk
}

fn rebuild_single(
    height_chunks: usize,
    position: ChunkPos,
    chunk: &Chunk,
    light: &ChunkLight,
    heightmap: &ChunkHeightmap,
) -> RebuiltChunkLight {
    let mut region = ChunkLightRegion::new(height_chunks);
    region.insert_target(position, chunk, light, heightmap);
    let mut rebuilt = region.rebuild();
    assert_eq!(rebuilt.len(), 1);
    rebuilt.pop().unwrap()
}

fn rebuilt_by_position(region: ChunkLightRegion<'_>) -> HashMap<ChunkPos, RebuiltChunkLight> {
    region
        .rebuild()
        .into_iter()
        .map(|rebuilt| (rebuilt.position, rebuilt))
        .collect()
}

#[test]
fn sky_light_above_an_opaque_surface_is_full() {
    let chunk = chunk_with_cells(|_, y, _| {
        if y < 10 {
            block_cell(BlockType::Stone)
        } else {
            ChunkCell::EMPTY
        }
    });
    let position = ChunkPos::new(-4, 0, -9);
    let rebuilt = rebuild_single(
        1,
        position,
        &chunk,
        &ChunkLight::default(),
        &ChunkHeightmap::default(),
    );

    assert_eq!(rebuilt.position, position);
    assert_eq!(rebuilt.heightmap.heights[8][8], 9);
    for y in 10..16 {
        assert_eq!(
            rebuilt.light.sky_light(local(8, y, 8)),
            SKY_LIGHT_MAX,
            "sky light above the surface at y={y}"
        );
    }
    assert_eq!(rebuilt.light.sky_light(local(8, 9, 8)), 0);
}

#[test]
fn sky_light_attenuates_through_transparent_blocks() {
    let chunk = chunk_with_cells(|_, y, _| {
        if y < 10 {
            block_cell(BlockType::Stone)
        } else if y < 13 {
            block_cell(BlockType::OakLeaves)
        } else {
            ChunkCell::EMPTY
        }
    });
    let rebuilt = rebuild_single(
        1,
        ChunkPos::new(-3, 0, 6),
        &chunk,
        &ChunkLight::default(),
        &ChunkHeightmap::default(),
    );

    assert_eq!(rebuilt.light.sky_light(local(0, 15, 0)), 15);
    assert_eq!(rebuilt.light.sky_light(local(0, 13, 0)), 15);
    assert_eq!(rebuilt.light.sky_light(local(0, 12, 0)), 14);
    assert_eq!(rebuilt.light.sky_light(local(0, 11, 0)), 13);
    assert_eq!(rebuilt.light.sky_light(local(0, 10, 0)), 12);
    assert_eq!(rebuilt.light.sky_light(local(0, 9, 0)), 0);
}

#[test]
fn sky_light_spreads_sideways_into_a_cave() {
    let chunk = chunk_with_cells(|x, y, z| {
        if z == 8 && (x == 0 || (x == 1 && (6..=10).contains(&y))) {
            ChunkCell::EMPTY
        } else {
            block_cell(BlockType::Stone)
        }
    });
    let rebuilt = rebuild_single(
        1,
        ChunkPos::new(-8, 0, -2),
        &chunk,
        &ChunkLight::default(),
        &ChunkHeightmap::default(),
    );

    assert_eq!(rebuilt.light.sky_light(local(0, 15, 8)), 15);
    let cave_light = rebuilt.light.sky_light(local(1, 8, 8));
    assert!(cave_light > 0);
    assert!(cave_light < SKY_LIGHT_MAX);
}

#[test]
fn block_light_emission_respects_transparent_and_opaque_cells() {
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(8, 8, 8, block_cell(BlockType::Glowstone));
    chunk.set_cell_xyz(7, 8, 8, block_cell(BlockType::OakLeaves));
    chunk.set_cell_xyz(9, 8, 8, block_cell(BlockType::Stone));

    let rebuilt = rebuild_single(
        1,
        ChunkPos::new(-12, 0, 5),
        &chunk,
        &ChunkLight::default(),
        &ChunkHeightmap::default(),
    );

    assert_eq!(rebuilt.light.block_light(local(8, 8, 8)), 15);
    assert_eq!(rebuilt.light.block_light(local(7, 8, 8)), 14);
    assert_eq!(rebuilt.light.block_light(local(6, 8, 8)), 13);
    assert_eq!(rebuilt.light.block_light(local(9, 8, 8)), 0);
}

#[test]
fn full_rebuild_removes_stale_light_without_harming_other_emitters() {
    let position = ChunkPos::new(-6, 0, -7);
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(4, 8, 4, block_cell(BlockType::Glowstone));
    chunk.set_cell_xyz(12, 8, 12, block_cell(BlockType::Glowstone));

    let first = rebuild_single(
        1,
        position,
        &chunk,
        &ChunkLight::default(),
        &ChunkHeightmap::default(),
    );
    assert_eq!(first.light.block_light(local(4, 8, 4)), 15);
    assert_eq!(first.light.block_light(local(12, 8, 12)), 15);

    chunk.set_cell_xyz(4, 8, 4, ChunkCell::EMPTY);
    let second = rebuild_single(1, position, &chunk, &first.light, &first.heightmap);
    assert!(second.light_changed());
    assert!(second.heightmap_changed());
    assert_eq!(second.light.block_light(local(4, 8, 4)), 0);
    assert_eq!(second.light.block_light(local(12, 8, 12)), 15);

    chunk.set_cell_xyz(12, 8, 12, ChunkCell::EMPTY);
    let third = rebuild_single(1, position, &chunk, &second.light, &second.heightmap);
    assert_eq!(third.light.block_light(local(4, 8, 4)), 0);
    assert_eq!(third.light.block_light(local(12, 8, 12)), 0);
}

#[test]
fn block_light_crosses_all_six_faces_at_absolute_negative_positions() {
    let source_position = ChunkPos::new(-7, 1, -11);
    let cases = [
        (IVec3::X, local(15, 8, 8), local(0, 8, 8)),
        (IVec3::NEG_X, local(0, 8, 8), local(15, 8, 8)),
        (IVec3::Y, local(8, 15, 8), local(8, 0, 8)),
        (IVec3::NEG_Y, local(8, 0, 8), local(8, 15, 8)),
        (IVec3::Z, local(8, 8, 15), local(8, 8, 0)),
        (IVec3::NEG_Z, local(8, 8, 0), local(8, 8, 15)),
    ];

    for (offset, source_local, neighbor_local) in cases {
        let mut source = Chunk::default();
        source.set_cell(source_local.as_uvec3(), block_cell(BlockType::Glowstone));
        let neighbor = Chunk::default();
        let neighbor_position = source_position.offset(offset);
        let source_light = ChunkLight::default();
        let neighbor_light = ChunkLight::default();
        let source_heightmap = ChunkHeightmap::default();
        let neighbor_heightmap = ChunkHeightmap::default();

        let mut region = ChunkLightRegion::new(4);
        region.insert_target(source_position, &source, &source_light, &source_heightmap);
        region.insert_target(
            neighbor_position,
            &neighbor,
            &neighbor_light,
            &neighbor_heightmap,
        );
        let rebuilt = rebuilt_by_position(region);

        assert_eq!(
            rebuilt[&source_position].light.block_light(source_local),
            15,
            "source for face offset {offset:?}"
        );
        assert_eq!(
            rebuilt[&neighbor_position]
                .light
                .block_light(neighbor_local),
            14,
            "neighbor for face offset {offset:?}"
        );
    }
}

#[test]
fn vertical_sky_occlusion_spans_an_entire_loaded_column() {
    let lower_position = ChunkPos::new(-9, 0, -4);
    let upper_position = ChunkPos::new(-9, 1, -4);
    let lower = Chunk::default();
    let mut upper = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            upper.set_cell_xyz(x, 0, z, block_cell(BlockType::Stone));
        }
    }
    let lower_light = ChunkLight::default();
    let upper_light = ChunkLight::default();
    let lower_heightmap = ChunkHeightmap::default();
    let upper_heightmap = ChunkHeightmap::default();

    let mut region = ChunkLightRegion::new(2);
    region.insert_target(lower_position, &lower, &lower_light, &lower_heightmap);
    region.insert_target(upper_position, &upper, &upper_light, &upper_heightmap);
    let rebuilt = rebuilt_by_position(region);

    assert_eq!(rebuilt[&upper_position].light.sky_light(local(8, 1, 8)), 15);
    assert_eq!(rebuilt[&upper_position].light.sky_light(local(8, 0, 8)), 0);
    assert_eq!(rebuilt[&lower_position].light.sky_light(local(8, 15, 8)), 0);
    for position in [lower_position, upper_position] {
        assert_eq!(rebuilt[&position].heightmap.heights[8][8], 16);
    }
}

#[test]
fn sky_light_waits_for_a_missing_top_chunk() {
    let position = ChunkPos::new(-2, 0, -13);
    let rebuilt = rebuild_single(
        2,
        position,
        &Chunk::default(),
        &ChunkLight::default(),
        &ChunkHeightmap::default(),
    );

    assert_eq!(rebuilt.light.sky_light(local(8, 15, 8)), 0);
}

#[test]
fn a_missing_middle_chunk_blocks_sky_until_the_column_is_complete() {
    let bottom_position = ChunkPos::new(-10, 0, -3);
    let middle_position = ChunkPos::new(-10, 1, -3);
    let top_position = ChunkPos::new(-10, 2, -3);
    let bottom = Chunk::default();
    let middle = Chunk::default();
    let top = Chunk::default();
    let bottom_light = ChunkLight::default();
    let top_light = ChunkLight::default();
    let bottom_heightmap = ChunkHeightmap::default();
    let top_heightmap = ChunkHeightmap::default();

    let mut incomplete = ChunkLightRegion::new(3);
    incomplete.insert_target(bottom_position, &bottom, &bottom_light, &bottom_heightmap);
    incomplete.insert_target(top_position, &top, &top_light, &top_heightmap);
    let first = rebuilt_by_position(incomplete);

    assert_eq!(first[&top_position].light.sky_light(local(8, 15, 8)), 15);
    assert_eq!(first[&bottom_position].light.sky_light(local(8, 15, 8)), 0);

    let middle_light = ChunkLight::default();
    let middle_heightmap = ChunkHeightmap::default();
    let mut complete = ChunkLightRegion::new(3);
    complete.insert_target(
        bottom_position,
        &bottom,
        &first[&bottom_position].light,
        &first[&bottom_position].heightmap,
    );
    complete.insert_target(middle_position, &middle, &middle_light, &middle_heightmap);
    complete.insert_target(
        top_position,
        &top,
        &first[&top_position].light,
        &first[&top_position].heightmap,
    );
    let second = rebuilt_by_position(complete);

    for position in [bottom_position, middle_position, top_position] {
        assert_eq!(
            second[&position].light.sky_light(local(8, 8, 8)),
            15,
            "completed column chunk {position:?}"
        );
    }
}

#[test]
fn boundary_light_is_read_only_and_only_targets_are_rebuilt() {
    let target_position = ChunkPos::new(-5, 0, -8);
    let boundary_position = target_position.offset(IVec3::NEG_X);
    let target = Chunk::default();
    let target_light = ChunkLight::default();
    let target_heightmap = ChunkHeightmap::default();
    let mut boundary_light = ChunkLight::default();
    boundary_light.set_block_light(local(15, 8, 8), 15);
    boundary_light.set_sky_light(local(15, 9, 8), 11);
    let boundary_before = boundary_light.clone();

    let mut region = ChunkLightRegion::new(2);
    region.insert_target(target_position, &target, &target_light, &target_heightmap);
    assert_eq!(
        region.required_boundary_positions(),
        HashSet::from([
            target_position.offset(IVec3::X),
            target_position.offset(IVec3::NEG_X),
            target_position.offset(IVec3::Y),
            target_position.offset(IVec3::NEG_Y),
            target_position.offset(IVec3::Z),
            target_position.offset(IVec3::NEG_Z),
        ])
    );
    region.insert_boundary_light(boundary_position, &boundary_light);
    let rebuilt = region.rebuild();

    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0].position, target_position);
    assert_eq!(rebuilt[0].light.block_light(local(0, 8, 8)), 14);
    assert_eq!(rebuilt[0].light.sky_light(local(0, 9, 8)), 10);
    assert_eq!(boundary_light, boundary_before);
}

#[test]
fn target_insertion_order_does_not_change_region_output() {
    let left_position = ChunkPos::new(-14, 0, -9);
    let right_position = left_position.offset(IVec3::X);
    let mut left = Chunk::default();
    left.set_cell_xyz(15, 8, 8, block_cell(BlockType::Glowstone));
    let right = Chunk::default();
    let left_light = ChunkLight::default();
    let right_light = ChunkLight::default();
    let left_heightmap = ChunkHeightmap::default();
    let right_heightmap = ChunkHeightmap::default();

    let mut left_first = ChunkLightRegion::new(1);
    left_first.insert_target(left_position, &left, &left_light, &left_heightmap);
    left_first.insert_target(right_position, &right, &right_light, &right_heightmap);
    let left_first = rebuilt_by_position(left_first)
        .into_iter()
        .map(|(position, rebuilt)| (position, (rebuilt.light, rebuilt.heightmap)))
        .collect::<HashMap<_, _>>();

    let mut right_first = ChunkLightRegion::new(1);
    right_first.insert_target(right_position, &right, &right_light, &right_heightmap);
    right_first.insert_target(left_position, &left, &left_light, &left_heightmap);
    let right_first = rebuilt_by_position(right_first)
        .into_iter()
        .map(|(position, rebuilt)| (position, (rebuilt.light, rebuilt.heightmap)))
        .collect::<HashMap<_, _>>();

    assert_eq!(left_first, right_first);
}

#[test]
fn rebuilding_identical_state_reports_no_changes() {
    let position = ChunkPos::new(-4, 0, -6);
    let chunk = chunk_with_cells(|_, y, _| {
        if y < 7 {
            block_cell(BlockType::Stone)
        } else if y == 10 {
            block_cell(BlockType::Glowstone)
        } else {
            ChunkCell::EMPTY
        }
    });
    let first = rebuild_single(
        1,
        position,
        &chunk,
        &ChunkLight::default(),
        &ChunkHeightmap::default(),
    );
    assert!(first.light_changed());
    assert!(first.heightmap_changed());

    let second = rebuild_single(1, position, &chunk, &first.light, &first.heightmap);
    assert!(!second.light_changed());
    assert!(!second.heightmap_changed());
    assert_eq!(second.light, first.light);
    assert_eq!(second.heightmap, first.heightmap);
}

#[test]
fn sixteen_chunk_height_preserves_the_highest_heightmap_value() {
    const HEIGHT_CHUNKS: usize = 16;

    let column = ChunkPos::new(-11, 0, -15);
    let mut chunks = (0..HEIGHT_CHUNKS)
        .map(|_| Chunk::default())
        .collect::<Vec<_>>();
    chunks[HEIGHT_CHUNKS - 1].set_cell_xyz(0, CHUNK_SIZE - 1, 0, block_cell(BlockType::Stone));
    let lights = (0..HEIGHT_CHUNKS)
        .map(|_| ChunkLight::default())
        .collect::<Vec<_>>();
    let heightmaps = (0..HEIGHT_CHUNKS)
        .map(|_| ChunkHeightmap::default())
        .collect::<Vec<_>>();

    let mut region = ChunkLightRegion::new(HEIGHT_CHUNKS);
    for y in 0..HEIGHT_CHUNKS {
        region.insert_target(
            ChunkPos::new(column.as_ivec3().x, y as i32, column.as_ivec3().z),
            &chunks[y],
            &lights[y],
            &heightmaps[y],
        );
    }
    let rebuilt = region.rebuild();

    assert_eq!(rebuilt.len(), HEIGHT_CHUNKS);
    assert!(
        rebuilt
            .iter()
            .all(|chunk| chunk.heightmap.heights[0][0] == u8::MAX)
    );
}

#[test]
#[should_panic]
fn heights_above_sixteen_chunks_are_rejected() {
    let _ = ChunkLightRegion::new(17);
}

#[test]
#[should_panic]
fn targets_outside_the_vertical_range_are_rejected() {
    let chunk = Chunk::default();
    let light = ChunkLight::default();
    let heightmap = ChunkHeightmap::default();
    let mut region = ChunkLightRegion::new(2);
    region.insert_target(ChunkPos::new(-3, 2, -5), &chunk, &light, &heightmap);
}

#[test]
fn packed_light_round_trips_through_a_local_position() {
    let mut light = ChunkLight::default();
    let position = local(7, 5, 3);
    light.set_sky_light(position, 13);
    light.set_block_light(position, 9);

    assert_eq!(light.sky_light(position), 13);
    assert_eq!(light.block_light(position), 9);
    assert_eq!(light.packed_light(position), 0xD9);
}
