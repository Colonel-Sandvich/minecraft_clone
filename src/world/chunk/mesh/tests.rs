use std::sync::Arc;

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::block::{
    BlockMaterialLayer, BlockType, WATER_RENDER_ID, from_render_id, render_id_for_block,
};
use crate::quad::Direction;
use crate::world::chunk::mesh::{binary, vertex_pulling};
use crate::world::chunk::{Chunk, ChunkCell, ChunkLight, ChunkNeedsLightUpload};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, ChunkMeshBlocks, ChunkNeedsMeshRebuild, ChunkPosition,
    VertexPullingLight, face_ao_from_indices, padded_chunk_index, water_corner_heights,
};

// Mirrors the removed mod.rs VERTEX_OFFSETS — only needed in test AO helpers.
const VERTEX_OFFSETS: [[IVec3; 4]; 6] = [
    [
        IVec3::new(0, 0, 1),
        IVec3::new(0, 0, 0),
        IVec3::new(0, 1, 1),
        IVec3::new(0, 1, 0),
    ],
    [
        IVec3::new(1, 0, 0),
        IVec3::new(1, 0, 1),
        IVec3::new(1, 1, 0),
        IVec3::new(1, 1, 1),
    ],
    [
        IVec3::new(0, 0, 1),
        IVec3::new(1, 0, 1),
        IVec3::new(0, 0, 0),
        IVec3::new(1, 0, 0),
    ],
    [
        IVec3::new(0, 1, 1),
        IVec3::new(0, 1, 0),
        IVec3::new(1, 1, 1),
        IVec3::new(1, 1, 0),
    ],
    [
        IVec3::new(0, 0, 0),
        IVec3::new(1, 0, 0),
        IVec3::new(0, 1, 0),
        IVec3::new(1, 1, 0),
    ],
    [
        IVec3::new(1, 0, 1),
        IVec3::new(0, 0, 1),
        IVec3::new(1, 1, 1),
        IVec3::new(0, 1, 1),
    ],
];

use strum::IntoEnumIterator;

// ---------------------------------------------------------------------------
// Legacy AO helpers (test-only — validates shared VERTEX_AO / occlusion logic)
// ---------------------------------------------------------------------------

fn vertex_ao(side1: bool, side2: bool, corner: bool) -> u8 {
    if side1 && side2 {
        0
    } else {
        3 - side1 as u8 - side2 as u8 - corner as u8
    }
}

fn ao_occludes(blocks: &ChunkMeshBlocks, x: i32, y: i32, z: i32) -> bool {
    block_occludes_ambient_light(get_block(blocks, x, y, z))
}

fn render_id_profile(rid: u16) -> Option<crate::block::BlockRenderProfile> {
    if rid == 0 {
        return None;
    }
    if rid == WATER_RENDER_ID {
        return Some(crate::block::BlockRenderProfile {
            layer: crate::block::BlockRenderLayer::Translucent,
            occlusion: crate::block::FaceOcclusion::None,
        });
    }
    from_render_id(rid).and_then(|b| b.render_profile())
}

fn block_occludes_ambient_light(cell: u16) -> bool {
    if cell == 0 || cell == WATER_RENDER_ID {
        return false;
    }
    from_render_id(cell)
        .and_then(|b| b.render_profile())
        .is_some_and(|profile| profile.occlusion == crate::block::FaceOcclusion::FullCube)
}

fn get_block(blocks: &ChunkMeshBlocks, x: i32, y: i32, z: i32) -> u16 {
    if !is_in_padded_chunk(x) || !is_in_padded_chunk(y) || !is_in_padded_chunk(z) {
        return 0;
    }
    let x = (x + 1) as usize;
    let y = (y + 1) as usize;
    let z = (z + 1) as usize;
    blocks.blocks[padded_chunk_index(x, y, z)]
}

fn block_cell(block: BlockType) -> ChunkCell {
    block.into()
}

fn is_in_padded_chunk(value: i32) -> bool {
    (-1..=CHUNK_ISIZE).contains(&value)
}

fn direction_offset(side: Direction) -> IVec3 {
    match side {
        Direction::Left => IVec3::NEG_X,
        Direction::Right => IVec3::X,
        Direction::Down => IVec3::NEG_Y,
        Direction::Up => IVec3::Y,
        Direction::Forward => IVec3::NEG_Z,
        Direction::Backward => IVec3::Z,
    }
}

fn tangent_axes(side: Direction) -> [usize; 2] {
    match side {
        Direction::Left | Direction::Right => [1, 2],
        Direction::Down | Direction::Up => [0, 2],
        Direction::Forward | Direction::Backward => [0, 1],
    }
}

fn sample_axis_offset(vertex_offset: IVec3, axis: usize) -> IVec3 {
    let sign = if axis_component(vertex_offset, axis) == 0 {
        -1
    } else {
        1
    };

    match axis {
        0 => IVec3::new(sign, 0, 0),
        1 => IVec3::new(0, sign, 0),
        2 => IVec3::new(0, 0, sign),
        _ => unreachable!(),
    }
}

fn axis_component(offset: IVec3, axis: usize) -> i32 {
    match axis {
        0 => offset.x,
        1 => offset.y,
        2 => offset.z,
        _ => unreachable!(),
    }
}

fn face_ao(blocks: &ChunkMeshBlocks, voxel: IVec3, side: Direction) -> [u8; 4] {
    let normal = direction_offset(side);
    let tangent_axes = tangent_axes(side);

    VERTEX_OFFSETS[side as usize].map(|vertex_offset| {
        let side1 = sample_axis_offset(vertex_offset, tangent_axes[0]);
        let side2 = sample_axis_offset(vertex_offset, tangent_axes[1]);

        vertex_ao(
            ao_occludes(
                blocks,
                voxel.x + normal.x + side1.x,
                voxel.y + normal.y + side1.y,
                voxel.z + normal.z + side1.z,
            ),
            ao_occludes(
                blocks,
                voxel.x + normal.x + side2.x,
                voxel.y + normal.y + side2.y,
                voxel.z + normal.z + side2.z,
            ),
            ao_occludes(
                blocks,
                voxel.x + normal.x + side1.x + side2.x,
                voxel.y + normal.y + side1.y + side2.y,
                voxel.z + normal.z + side1.z + side2.z,
            ),
        )
    })
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn padded_chunk_blocks<'a>(
    chunks: impl IntoIterator<Item = (IVec3, &'a Chunk)>,
) -> ChunkMeshBlocks {
    let chunks = chunks.into_iter().collect::<HashMap<_, _>>();
    ChunkMeshBlocks::from_chunks(IVec3::ZERO, &chunks)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn vertex_ao_uses_four_symmetric_levels() {
    let cases = [
        ((false, false, false), 3),
        ((true, false, false), 2),
        ((false, true, false), 2),
        ((false, false, true), 2),
        ((true, false, true), 1),
        ((false, true, true), 1),
        ((true, true, false), 0),
        ((true, true, true), 0),
    ];

    for ((side1, side2, corner), expected) in cases {
        assert_eq!(vertex_ao(side1, side2, corner), expected);
    }
}

#[test]
fn face_ao_samples_adjacent_plane_and_only_full_cube_occluders() {
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(1, 1, 1, block_cell(BlockType::Stone));

    chunk.set_cell_xyz(0, 2, 1, block_cell(BlockType::Stone));
    chunk.set_cell_xyz(1, 2, 2, block_cell(BlockType::Stone));
    chunk.set_cell_xyz(0, 2, 2, block_cell(BlockType::Stone));
    chunk.set_cell_xyz(2, 2, 1, block_cell(BlockType::Glass));
    chunk.set_cell_xyz(2, 2, 2, block_cell(BlockType::OakLeaves));

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    assert_eq!(
        face_ao(&blocks, IVec3::new(1, 1, 1), Direction::Up),
        [0, 2, 2, 3]
    );
}

#[test]
fn face_ao_samples_loaded_face_neighbor_chunk() {
    let mut centre = Chunk::default();
    centre.set_cell_xyz(1, 15, 1, block_cell(BlockType::Stone));

    let mut above = Chunk::default();
    above.set_cell_xyz(0, 0, 1, block_cell(BlockType::Stone));
    above.set_cell_xyz(1, 0, 2, block_cell(BlockType::Stone));
    above.set_cell_xyz(0, 0, 2, block_cell(BlockType::Stone));

    let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (IVec3::Y, &above)]);

    assert_eq!(
        face_ao(&padded_blocks, IVec3::new(1, 15, 1), Direction::Up),
        [0, 2, 2, 3]
    );
}

#[test]
fn face_ao_samples_loaded_edge_neighbor_chunk() {
    let mut centre = Chunk::default();
    centre.set_cell_xyz(0, 15, 1, block_cell(BlockType::Stone));

    let mut edge = Chunk::default();
    edge.set_cell_xyz(15, 0, 1, block_cell(BlockType::Stone));
    edge.set_cell_xyz(15, 0, 2, block_cell(BlockType::Stone));

    let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (ivec3(-1, 1, 0), &edge)]);

    assert_eq!(
        face_ao(&padded_blocks, IVec3::new(0, 15, 1), Direction::Up),
        [1, 2, 3, 3]
    );
}

#[test]
fn face_ao_samples_loaded_corner_neighbor_chunk() {
    let mut centre = Chunk::default();
    centre.set_cell_xyz(0, 15, 15, block_cell(BlockType::Stone));

    let mut corner = Chunk::default();
    corner.set_cell_xyz(15, 0, 0, block_cell(BlockType::Stone));

    let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (ivec3(-1, 1, 1), &corner)]);

    assert_eq!(
        face_ao(&padded_blocks, IVec3::new(0, 15, 15), Direction::Up),
        [2, 3, 3, 3]
    );
}

#[test]
fn indexed_face_ao_matches_reference_corner_order_for_all_directions() {
    let mut chunk = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let hash = x * 17 + y * 31 + z * 43;
                let cell = match hash % 5 {
                    0 | 3 => block_cell(BlockType::Stone),
                    1 => block_cell(BlockType::Glass),
                    _ => ChunkCell::EMPTY,
                };
                chunk.set_cell_xyz(x, y, z, cell);
            }
        }
    }

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let padded_index = padded_chunk_index(x + 1, y + 1, z + 1);
                let voxel = IVec3::new(x as i32, y as i32, z as i32);

                for side in Direction::iter() {
                    assert_eq!(
                        face_ao_from_indices(&blocks, padded_index, side as usize),
                        face_ao(&blocks, voxel, side),
                        "voxel={voxel:?} side={side:?}",
                    );
                }
            }
        }
    }
}

#[test]
fn mesh_rebuild_marker_is_removed_after_rebuild() {
    let mut app = mesh_rebuild_app();

    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(0, 0, 0, block_cell(BlockType::Stone));
    let chunk_entity = app
        .world_mut()
        .spawn((ChunkPosition(IVec3::ZERO), chunk, ChunkNeedsMeshRebuild))
        .id();

    app.update();

    let world = app.world();
    assert!(world.get::<ChunkNeedsMeshRebuild>(chunk_entity).is_none());
    let children = world.get::<Children>(chunk_entity).unwrap();
    let mesh_child_count = children
        .iter()
        .filter(|child| world.get::<super::VertexPullingMesh>(*child).is_some())
        .count();
    assert_eq!(mesh_child_count, 1);
}

#[test]
fn light_upload_marker_updates_existing_vertex_pulling_light() {
    let mut app = light_upload_app();

    let mut chunk_light = ChunkLight::default();
    chunk_light.set_sky_light(uvec3(0, 0, 0), 10);
    chunk_light.set_block_light(uvec3(0, 0, 0), 15);
    let expected_light_data = ChunkLight::build_padded_light_data(
        IVec3::ZERO,
        &HashMap::from([(IVec3::ZERO, &chunk_light)]),
    );

    let chunk_entity = app
        .world_mut()
        .spawn((
            ChunkPosition(IVec3::ZERO),
            Chunk::default(),
            chunk_light,
            ChunkNeedsLightUpload,
        ))
        .id();
    let child_entity = spawn_light_child(app.world_mut(), chunk_entity, empty_light_data());
    let sibling_child_entity = spawn_light_child(app.world_mut(), chunk_entity, empty_light_data());

    app.update();

    let world = app.world();
    assert!(world.get::<ChunkNeedsLightUpload>(chunk_entity).is_none());
    assert!(world.get::<ChunkNeedsMeshRebuild>(chunk_entity).is_none());
    let child_light = world.get::<VertexPullingLight>(child_entity).unwrap();
    let sibling_child_light = world
        .get::<VertexPullingLight>(sibling_child_entity)
        .unwrap();
    assert_eq!(
        child_light.light_data.as_ref(),
        expected_light_data.as_ref()
    );
    assert_eq!(
        sibling_child_light.light_data.as_ref(),
        expected_light_data.as_ref()
    );
    assert!(Arc::ptr_eq(
        &child_light.light_data,
        &sibling_child_light.light_data
    ));
}

#[test]
fn mesh_rebuild_new_layer_child_reuses_existing_light_data() {
    let mut app = mesh_rebuild_app();

    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(0, 0, 0, block_cell(BlockType::Stone));
    chunk.set_cell_xyz(1, 0, 0, block_cell(BlockType::Glass));
    let existing_light_data = empty_light_data();

    let chunk_entity = app
        .world_mut()
        .spawn((ChunkPosition(IVec3::ZERO), chunk, ChunkNeedsMeshRebuild))
        .id();
    spawn_vp_layer_child(
        app.world_mut(),
        chunk_entity,
        BlockMaterialLayer::Opaque,
        existing_light_data.clone(),
    );

    app.update();

    let world = app.world();
    assert!(world.get::<ChunkNeedsMeshRebuild>(chunk_entity).is_none());
    let children = world.get::<Children>(chunk_entity).unwrap();
    let child_light_data = children
        .iter()
        .filter_map(|child| world.get::<VertexPullingLight>(child))
        .map(|light| light.light_data.clone())
        .collect::<Vec<_>>();

    assert_eq!(child_light_data.len(), 2);
    assert!(
        child_light_data
            .iter()
            .all(|light_data| Arc::ptr_eq(light_data, &existing_light_data))
    );
}

fn mesh_rebuild_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .insert_resource(test_texture_map())
        .insert_resource(crate::textures::BlockTextures::test_handles())
        .add_systems(Update, super::rebuild_chunk_meshes);
    app
}

fn light_upload_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_systems(Update, super::upload_chunk_lights);
    app
}

fn empty_light_data() -> Arc<[u32]> {
    ChunkLight::build_padded_light_data(IVec3::ZERO, &HashMap::default()).into()
}

fn spawn_light_child(world: &mut World, chunk_entity: Entity, light_data: Arc<[u32]>) -> Entity {
    world
        .spawn((ChildOf(chunk_entity), VertexPullingLight { light_data }))
        .id()
}

fn spawn_vp_layer_child(
    world: &mut World,
    chunk_entity: Entity,
    layer: BlockMaterialLayer,
    light_data: Arc<[u32]>,
) -> Entity {
    world
        .spawn((
            ChildOf(chunk_entity),
            super::ChunkMaterialLayerMarker(layer),
            super::VertexPullingMesh {
                face_count: 0,
                material_layer: layer,
                chunk_origin: Vec3::ZERO,
            },
            VertexPullingLight { light_data },
        ))
        .id()
}

#[test]
fn vertex_pulling_descriptors_match_reference_face_counts() {
    for case in test_chunks() {
        let blocks = ChunkMeshBlocks::from_chunk(&case.chunk);
        let reference_faces: Vec<_> = reference_face_counts(&blocks);
        let descriptor_faces: Vec<_> = vertex_pulling::build_descriptors(&blocks)
            .into_iter()
            .map(|(layer, descriptors)| (layer, descriptors.len()))
            .collect();

        assert_eq!(reference_faces, descriptor_faces, "{}", case.name);
    }
}

#[test]
fn water_top_descriptor_packs_flow_direction() {
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(8, 1, 8, ChunkCell::water_source());
    chunk.set_cell_xyz(9, 1, 8, ChunkCell::water_flow(7));
    chunk.set_cell_xyz(7, 1, 8, block_cell(BlockType::Stone));
    chunk.set_cell_xyz(8, 1, 7, block_cell(BlockType::Stone));
    chunk.set_cell_xyz(8, 1, 9, block_cell(BlockType::Stone));

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let descriptor = vertex_pulling::build_descriptors(&blocks)
        .into_iter()
        .flat_map(|(_, descriptors)| descriptors)
        .find(|desc| {
            desc.block_type() == WATER_RENDER_ID as u32
                && desc.x() == 8
                && desc.y() == 1
                && desc.z() == 8
                && desc.face_dir() == Direction::Up as u32
        })
        .expect("source water top face should be emitted");

    assert!(descriptor.water_up_flowing());
    assert_eq!(descriptor.water_flow_code(), 1);
}

#[test]
fn shallow_water_descriptor_marks_zero_height_water_geometry() {
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(8, 1, 8, ChunkCell::water_flow(1));

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let descriptor = vertex_pulling::build_descriptors(&blocks)
        .into_iter()
        .flat_map(|(_, descriptors)| descriptors)
        .find(|desc| {
            desc.block_type() == WATER_RENDER_ID as u32
                && desc.x() == 8
                && desc.y() == 1
                && desc.z() == 8
                && desc.face_dir() == Direction::Up as u32
        })
        .expect("shallow water top face should be emitted");

    assert_eq!(descriptor.info >> 16, 0);
    assert!(descriptor.has_water_geometry());
}

#[test]
fn water_corner_heights_use_vanilla_ninths_and_full_columns() {
    let mut chunk = Chunk::default();
    for x in 7..=9 {
        for z in 7..=9 {
            chunk.set_cell_xyz(x, 1, z, ChunkCell::water_source());
        }
    }

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let center = padded_chunk_index(9, 2, 9);
    assert_eq!(water_corner_heights(8, &blocks, center), (8, 8, 8, 8));

    chunk.set_cell_xyz(8, 2, 8, ChunkCell::water_source());
    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    assert_eq!(water_corner_heights(8, &blocks, center), (9, 9, 9, 9));
}

#[test]
fn hybrid_mesher_matches_scalar_water_flow_descriptors() {
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(8, 1, 8, ChunkCell::water_source());
    chunk.set_cell_xyz(9, 1, 8, ChunkCell::water_flow(7));
    chunk.set_cell_xyz(8, 1, 9, ChunkCell::water_flow(6));

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    assert_eq!(
        vertex_pulling::build_descriptors(&blocks),
        binary::build_descriptors_hybrid(&blocks)
    );
}

// ---------------------------------------------------------------------------
// Test helpers for face-count validation
// ---------------------------------------------------------------------------

fn reference_face_counts(blocks: &ChunkMeshBlocks) -> Vec<(BlockMaterialLayer, usize)> {
    let mut counts = [0usize; BlockMaterialLayer::COUNT];
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let cell = get_block(blocks, x as i32, y as i32, z as i32);
                let Some(profile) = render_id_profile(cell) else {
                    continue;
                };

                let neighbors = [
                    get_block(blocks, x as i32 - 1, y as i32, z as i32),
                    get_block(blocks, x as i32 + 1, y as i32, z as i32),
                    get_block(blocks, x as i32, y as i32 - 1, z as i32),
                    get_block(blocks, x as i32, y as i32 + 1, z as i32),
                    get_block(blocks, x as i32, y as i32, z as i32 - 1),
                    get_block(blocks, x as i32, y as i32, z as i32 + 1),
                ];

                for neighbor in neighbors {
                    let Some(neighbor_profile) = render_id_profile(neighbor) else {
                        counts[profile.material_layer().index()] += 1;
                        continue;
                    };

                    if neighbor_profile.occlusion == crate::block::FaceOcclusion::FullCube {
                        continue;
                    }

                    if cell == neighbor
                        && profile.occlusion == crate::block::FaceOcclusion::None
                        && neighbor_profile.occlusion == crate::block::FaceOcclusion::None
                        && cell != render_id_for_block(BlockType::OakLeaves)
                    {
                        continue;
                    }

                    counts[profile.material_layer().index()] += 1;
                }
            }
        }
    }

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            let c = counts[layer.index()];
            (c > 0).then_some((layer, c))
        })
        .collect()
}

fn test_texture_map() -> crate::block::BlockTextureMap {
    use crate::block::{RENDER_ID_COUNT, render_id_to_texture_path};
    let mut paths = HashMap::default();
    for rid in 1..RENDER_ID_COUNT as u16 {
        for side in Direction::iter() {
            let path = render_id_to_texture_path(rid, side).to_owned();
            let next_layer = paths.len() as u32;
            paths
                .entry(path)
                .or_insert(crate::block::BlockTextureAnimation::new(
                    crate::block::BlockTextureLayer::new(next_layer),
                    1,
                ));
        }
    }
    crate::block::BlockTextureMap(paths)
}

struct TestChunkCase {
    name: &'static str,
    chunk: Chunk,
}

fn test_chunks() -> Vec<TestChunkCase> {
    let mut single = Chunk::default();
    single.set_cell_xyz(8, 8, 8, block_cell(BlockType::Stone));

    let mut checkerboard = Chunk::default();
    let mut mixed = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                if (x + y + z) % 2 == 0 {
                    checkerboard.set_cell_xyz(x, y, z, block_cell(BlockType::Stone));
                }

                let cell = if y < 4 {
                    block_cell(BlockType::Stone)
                } else if (x + z) % 7 == 0 {
                    block_cell(BlockType::Glass)
                } else if (x * 3 + y + z * 5) % 11 == 0 {
                    block_cell(BlockType::OakLeaves)
                } else {
                    ChunkCell::EMPTY
                };
                mixed.set_cell_xyz(x, y, z, cell);
            }
        }
    }

    let leaves = Chunk::filled(block_cell(BlockType::OakLeaves));

    let empty = Chunk::default();

    let full_stone = Chunk::filled(block_cell(BlockType::Stone));

    let all_glass = Chunk::filled(block_cell(BlockType::Glass));

    let mut water_basin = Chunk::filled(block_cell(BlockType::Stone));
    for x in 4..12 {
        for z in 4..12 {
            water_basin.set_cell_xyz(x, 8, z, ChunkCell::water_source());
        }
    }
    water_basin.set_cell_xyz(8, 9, 8, block_cell(BlockType::Ice));

    vec![
        TestChunkCase {
            name: "empty",
            chunk: empty,
        },
        TestChunkCase {
            name: "full_stone",
            chunk: full_stone,
        },
        TestChunkCase {
            name: "all_glass",
            chunk: all_glass,
        },
        TestChunkCase {
            name: "water_basin",
            chunk: water_basin,
        },
        TestChunkCase {
            name: "single",
            chunk: single,
        },
        TestChunkCase {
            name: "checkerboard",
            chunk: checkerboard,
        },
        TestChunkCase {
            name: "mixed",
            chunk: mixed,
        },
        TestChunkCase {
            name: "leaves",
            chunk: leaves,
        },
    ]
}
