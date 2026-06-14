use bevy::{math::Rect, mesh::VertexAttributeValues, platform::collections::HashMap, prelude::*};
use strum::IntoEnumIterator;

use crate::block::{BlockMaterialLayer, BlockType, block_and_side_to_texture_path};
use crate::quad::{Direction, Quad, QuadGroups};
use crate::world::chunk::ambient_occlusion::AO_BRIGHTNESS;
use crate::world::chunk::mesh::reference::{
    face_ao, make_layered_quad_groups_from_blocks, vertex_ao,
};
use crate::world::chunk::mesh::{
    ChunkMeshBlocks, ChunkMeshInput, ChunkMesher, make_chunk_meshes,
    make_mesh_from_quad_groups_with_ao_brightness,
};
use crate::world::chunk::{CHUNK_SIZE, Chunk, ChunkNeedsMeshRebuild, ChunkPosition};

use super::{
    DirectChunkMesher, FullCubeShellChunkMesher, GreedyChunkMesher, HybridChunkMesher,
    PartitionedGreedyChunkMesher, ReferenceChunkMesher, SweepChunkMesher,
};
use super::{
    GROUND_BOUNCE_FACE_BRIGHTNESS, HORIZON_FACE_BRIGHTNESS, SKY_FACE_BRIGHTNESS, face_brightness,
};

fn test_texture_map() -> crate::block::BlockTextureMap {
    let mut paths = HashMap::default();

    for block in BlockType::iter() {
        if block == BlockType::Air {
            continue;
        }

        for side in Direction::iter() {
            paths.insert(
                block_and_side_to_texture_path(block, side).to_owned(),
                Rect::new(0.0, 0.0, 1.0, 1.0),
            );
        }
    }

    crate::block::BlockTextureMap(paths)
}

fn quad_count(groups: &QuadGroups) -> usize {
    groups.groups.iter().map(Vec::len).sum()
}

fn mesh_signature(
    meshes: Vec<(BlockMaterialLayer, Mesh)>,
) -> Vec<(BlockMaterialLayer, usize, usize)> {
    meshes
        .into_iter()
        .map(|(layer, mesh)| {
            let vertex_count = match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
                VertexAttributeValues::Float32x3(values) => values.len(),
                values => panic!("unexpected position attribute: {values:?}"),
            };
            let index_count = match mesh.indices().unwrap() {
                bevy::mesh::Indices::U16(values) => values.len(),
                bevy::mesh::Indices::U32(values) => values.len(),
            };

            (layer, vertex_count, index_count)
        })
        .collect()
}

fn padded_chunk_blocks<'a>(
    chunks: impl IntoIterator<Item = (IVec3, &'a Chunk)>,
) -> ChunkMeshBlocks {
    let chunks = chunks.into_iter().collect::<HashMap<_, _>>();
    ChunkMeshBlocks::from_chunks(IVec3::ZERO, &chunks)
}

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

struct TestChunkCase {
    name: &'static str,
    chunk: Chunk,
}

fn test_chunks() -> Vec<TestChunkCase> {
    let mut single = Chunk::default();
    single.blocks[8][8][8] = BlockType::Stone;

    let mut checkerboard = Chunk::default();
    let mut mixed = Chunk::default();
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                if (x + y + z) % 2 == 0 {
                    checkerboard.blocks[x][z][y] = BlockType::Stone;
                }

                mixed.blocks[x][z][y] = if y < 4 {
                    BlockType::Stone
                } else if (x + z) % 7 == 0 {
                    BlockType::Glass
                } else if (x * 3 + y + z * 5) % 11 == 0 {
                    BlockType::OakLeaves
                } else {
                    BlockType::Air
                };
            }
        }
    }

    let mut leaves = Chunk::default();
    leaves.blocks = [[[BlockType::OakLeaves; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];

    let empty = Chunk::default();

    let mut full_stone = Chunk::default();
    full_stone.blocks = [[[BlockType::Stone; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];

    let mut all_glass = Chunk::default();
    all_glass.blocks = [[[BlockType::Glass; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];

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

#[test]
fn all_fast_meshers_match_direct_for_all_chunks() {
    let texture_map = test_texture_map();

    for case in test_chunks() {
        let blocks = ChunkMeshBlocks::from_chunk(&case.chunk);
        let input = ChunkMeshInput {
            blocks: &blocks,
            block_texture_map: &texture_map,
            ao_brightness: AO_BRIGHTNESS,
        };

        let direct = mesh_signature(DirectChunkMesher.mesh(input));
        assert_eq!(
            direct,
            mesh_signature(HybridChunkMesher.mesh(input)),
            "hybrid vs direct: {}",
            case.name
        );
    }
}

#[test]
fn all_meshers_match_reference_for_all_chunks() {
    let texture_map = test_texture_map();

    for case in test_chunks() {
        let blocks = ChunkMeshBlocks::from_chunk(&case.chunk);
        let input = ChunkMeshInput {
            blocks: &blocks,
            block_texture_map: &texture_map,
            ao_brightness: AO_BRIGHTNESS,
        };

        let reference = mesh_signature(ReferenceChunkMesher.mesh(input));
        assert_eq!(
            reference,
            mesh_signature(DirectChunkMesher.mesh(input)),
            "direct vs reference: {}",
            case.name
        );
        assert_eq!(
            reference,
            mesh_signature(HybridChunkMesher.mesh(input)),
            "hybrid vs reference: {}",
            case.name
        );
        assert_eq!(
            reference,
            mesh_signature(SweepChunkMesher.mesh(input)),
            "sweep vs reference: {}",
            case.name
        );
    }
}

#[test]
fn greedy_matches_direct_for_checkerboard_no_adjacent_faces_to_merge() {
    let texture_map = test_texture_map();
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

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let input = ChunkMeshInput {
        blocks: &blocks,
        block_texture_map: &texture_map,
        ao_brightness: AO_BRIGHTNESS,
    };

    assert_eq!(
        mesh_signature(DirectChunkMesher.mesh(input)),
        mesh_signature(GreedyChunkMesher.mesh(input)),
        "greedy should match direct for checkerboard (no adjacent same-type faces to merge)"
    );
}

#[test]
fn greedy_has_fewer_vertices_than_direct_for_full_stone() {
    let texture_map = test_texture_map();
    let mut chunk = Chunk::default();
    chunk.blocks = [[[BlockType::Stone; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let input = ChunkMeshInput {
        blocks: &blocks,
        block_texture_map: &texture_map,
        ao_brightness: AO_BRIGHTNESS,
    };

    let direct_sig = mesh_signature(DirectChunkMesher.mesh(input));
    let greedy_sig = mesh_signature(GreedyChunkMesher.mesh(input));

    assert_eq!(direct_sig.len(), greedy_sig.len(), "same number of layers");
    for ((dlayer, dverts, _), (glayer, gverts, _)) in direct_sig.iter().zip(greedy_sig.iter()) {
        assert_eq!(dlayer, glayer, "same layer order");
        assert!(
            gverts <= dverts,
            "greedy ({gverts}) should have <= vertices than direct ({dverts}) for layer {dlayer:?}"
        );
    }
}

#[test]
fn greedy_does_not_crash_on_any_test_chunk() {
    let texture_map = test_texture_map();

    for case in test_chunks() {
        let blocks = ChunkMeshBlocks::from_chunk(&case.chunk);
        let input = ChunkMeshInput {
            blocks: &blocks,
            block_texture_map: &texture_map,
            ao_brightness: AO_BRIGHTNESS,
        };

        let result = GreedyChunkMesher.mesh(input);
        let direct_result = DirectChunkMesher.mesh(input);

        assert_eq!(
            result.len(),
            direct_result.len(),
            "greedy should produce same number of layers as direct for {}",
            case.name
        );
    }
}

#[test]
fn shell_mesh_counts_match_reference_for_full_stone() {
    let texture_map = test_texture_map();
    let mut full_stone = Chunk::default();
    full_stone.blocks = [[[BlockType::Stone; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE];

    let blocks = ChunkMeshBlocks::from_chunk(&full_stone);
    let input = ChunkMeshInput {
        blocks: &blocks,
        block_texture_map: &texture_map,
        ao_brightness: AO_BRIGHTNESS,
    };

    assert_eq!(
        mesh_signature(ReferenceChunkMesher.mesh(input)),
        mesh_signature(FullCubeShellChunkMesher.mesh(input)),
        "shell vs reference for full_stone"
    );
}

#[test]
fn face_ao_samples_adjacent_plane_and_only_full_cube_occluders() {
    let mut chunk = Chunk::default();
    chunk.blocks[1][1][1] = BlockType::Stone;

    chunk.blocks[0][1][2] = BlockType::Stone;
    chunk.blocks[1][2][2] = BlockType::Stone;
    chunk.blocks[0][2][2] = BlockType::Stone;
    chunk.blocks[2][1][2] = BlockType::Glass;
    chunk.blocks[2][2][2] = BlockType::OakLeaves;

    assert_eq!(
        face_ao(&chunk, IVec3::new(1, 1, 1), Direction::Up),
        [0, 2, 2, 3]
    );
}

#[test]
fn face_ao_samples_loaded_face_neighbor_chunk() {
    let mut centre = Chunk::default();
    centre.blocks[1][1][15] = BlockType::Stone;

    let mut above = Chunk::default();
    above.blocks[0][1][0] = BlockType::Stone;
    above.blocks[1][2][0] = BlockType::Stone;
    above.blocks[0][2][0] = BlockType::Stone;

    let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (IVec3::Y, &above)]);

    assert_eq!(
        face_ao(&padded_blocks, IVec3::new(1, 15, 1), Direction::Up),
        [0, 2, 2, 3]
    );
}

#[test]
fn face_ao_samples_loaded_edge_neighbor_chunk() {
    let mut centre = Chunk::default();
    centre.blocks[0][1][15] = BlockType::Stone;

    let mut edge = Chunk::default();
    edge.blocks[15][1][0] = BlockType::Stone;
    edge.blocks[15][2][0] = BlockType::Stone;

    let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (ivec3(-1, 1, 0), &edge)]);

    assert_eq!(
        face_ao(&padded_blocks, IVec3::new(0, 15, 1), Direction::Up),
        [1, 2, 3, 3]
    );
}

#[test]
fn face_ao_samples_loaded_corner_neighbor_chunk() {
    let mut centre = Chunk::default();
    centre.blocks[0][15][15] = BlockType::Stone;

    let mut corner = Chunk::default();
    corner.blocks[15][0][0] = BlockType::Stone;

    let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (ivec3(-1, 1, 1), &corner)]);

    assert_eq!(
        face_ao(&padded_blocks, IVec3::new(0, 15, 15), Direction::Up),
        [2, 3, 3, 3]
    );
}

#[test]
fn boundary_faces_are_culled_against_loaded_neighbor_chunks() {
    let texture_map = test_texture_map();
    let mut centre = Chunk::default();
    centre.blocks[15][0][0] = BlockType::Stone;

    let mut right = Chunk::default();
    right.blocks[0][0][0] = BlockType::Stone;

    let padded_blocks = padded_chunk_blocks([(IVec3::ZERO, &centre), (IVec3::X, &right)]);
    let groups = make_layered_quad_groups_from_blocks(&padded_blocks, &texture_map);
    let opaque_groups = &groups.layers[BlockMaterialLayer::Opaque.index()];

    assert_eq!(quad_count(opaque_groups), 5);
    assert_eq!(opaque_groups.groups[Direction::Right as usize].len(), 0);
}

#[test]
fn mesh_bakes_ao_into_colours_and_chooses_less_biased_diagonal() {
    let mut groups = QuadGroups::default();
    let color = Vec4::new(0.8, 0.6, 0.4, 0.5);
    groups.groups[Direction::Up as usize].push(Quad {
        voxel: UVec3::ZERO,
        color,
        uv: Rect::new(0.0, 0.0, 1.0, 1.0),
        ao: [3, 0, 0, 3],
    });

    let mesh = make_mesh_from_quad_groups_with_ao_brightness(&groups, AO_BRIGHTNESS).unwrap();
    let Some(VertexAttributeValues::Float32x4(colours)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("missing colour attribute");
    };
    let dark = AO_BRIGHTNESS[0];
    assert_eq!(
        colours,
        &vec![
            [0.8, 0.6, 0.4, 0.5],
            [0.8 * dark, 0.6 * dark, 0.4 * dark, 0.5],
            [0.8 * dark, 0.6 * dark, 0.4 * dark, 0.5],
            [0.8, 0.6, 0.4, 0.5],
        ]
    );

    let Some(bevy::mesh::Indices::U32(indices)) = mesh.indices() else {
        panic!("missing indices");
    };
    assert_eq!(indices, &[0, 3, 1, 0, 2, 3]);
}

#[test]
fn face_lighting_uses_hemisphere_levels_not_horizontal_fake_sun() {
    assert_eq!(face_brightness(Direction::Up), SKY_FACE_BRIGHTNESS);
    assert_eq!(
        face_brightness(Direction::Down),
        GROUND_BOUNCE_FACE_BRIGHTNESS
    );

    for side in [
        Direction::Left,
        Direction::Right,
        Direction::Forward,
        Direction::Backward,
    ] {
        assert_eq!(face_brightness(side), HORIZON_FACE_BRIGHTNESS);
    }
}

#[test]
fn mesh_bakes_face_lighting_and_ao_into_colours() {
    let mut groups = QuadGroups::default();
    let color = Vec4::new(1.0, 0.5, 0.25, 0.75);
    groups.groups[Direction::Right as usize].push(Quad {
        voxel: UVec3::ZERO,
        color,
        uv: Rect::new(0.0, 0.0, 1.0, 1.0),
        ao: [3, 2, 1, 0],
    });

    let ao_brightness = [0.25, 0.5, 0.75, 1.0];
    let mesh = make_mesh_from_quad_groups_with_ao_brightness(&groups, ao_brightness).unwrap();
    let Some(VertexAttributeValues::Float32x4(colours)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("missing colour attribute");
    };

    let face_light = HORIZON_FACE_BRIGHTNESS;
    assert_eq!(
        colours,
        &vec![
            [1.0 * face_light, 0.5 * face_light, 0.25 * face_light, 0.75],
            [
                1.0 * face_light * 0.75,
                0.5 * face_light * 0.75,
                0.25 * face_light * 0.75,
                0.75,
            ],
            [
                1.0 * face_light * 0.5,
                0.5 * face_light * 0.5,
                0.25 * face_light * 0.5,
                0.75,
            ],
            [
                1.0 * face_light * 0.25,
                0.5 * face_light * 0.25,
                0.25 * face_light * 0.25,
                0.75,
            ],
        ]
    );
}

#[test]
fn mesh_rebuild_marker_is_removed_after_rebuild() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<crate::world::chunk::ambient_occlusion::AmbientOcclusionSettings>()
        .insert_resource(test_texture_map())
        .insert_resource(crate::textures::BlockStandardMaterials::test_handles())
        .add_systems(Update, super::rebuild_chunk_meshes);

    let mut chunk = Chunk::default();
    chunk.blocks[0][0][0] = BlockType::Stone;
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
        .filter(|child| world.get::<Mesh3d>(*child).is_some())
        .count();
    assert_eq!(mesh_child_count, 1);
}

#[test]
fn adjacent_leaves_emit_internal_faces_both_directions() {
    let texture_map = test_texture_map();
    let mut chunk = Chunk::default();
    chunk.blocks[0][0][0] = BlockType::OakLeaves;
    chunk.blocks[1][0][0] = BlockType::OakLeaves;

    let groups = make_layered_quad_groups_from_blocks(&chunk, &texture_map);
    let leaf_groups = &groups.layers[BlockMaterialLayer::Cutout.index()];

    assert_eq!(quad_count(leaf_groups), 12);
    assert_eq!(
        groups.layers[BlockMaterialLayer::Opaque.index()]
            .groups
            .iter()
            .map(Vec::len)
            .sum::<usize>(),
        0
    );
    assert_eq!(leaf_groups.groups[Direction::Right as usize].len(), 2);
    assert_eq!(leaf_groups.groups[Direction::Left as usize].len(), 2);
}

#[test]
fn leaves_do_not_occlude_opaque_faces() {
    let texture_map = test_texture_map();
    let mut chunk = Chunk::default();
    chunk.blocks[4][4][4] = BlockType::Stone;
    chunk.blocks[5][4][4] = BlockType::OakLeaves;

    let groups = make_layered_quad_groups_from_blocks(&chunk, &texture_map);
    let opaque_group = &groups.layers[BlockMaterialLayer::Opaque.index()];

    assert_eq!(
        opaque_group.groups[Direction::Right as usize].len(),
        1,
        "stone face next to leaves should emit"
    );
}

#[test]
fn chunk_meshes_are_split_by_render_layer() {
    let texture_map = test_texture_map();
    let mut chunk = Chunk::default();
    chunk.blocks[4][4][4] = BlockType::Stone;
    chunk.blocks[4][4][5] = BlockType::OakLeaves;
    chunk.blocks[4][4][6] = BlockType::Glass;

    let meshes = make_chunk_meshes(&chunk, &texture_map);
    assert_eq!(meshes.len(), 2, "should produce two layer meshes");

    let layers: Vec<_> = meshes.iter().map(|(layer, _)| *layer).collect();
    assert!(layers.contains(&BlockMaterialLayer::Opaque));
    assert!(layers.contains(&BlockMaterialLayer::Cutout));
}

#[test]
fn greedy_uv0_spans_merged_quads() {
    let mut texture_map = test_texture_map();
    texture_map.0.insert(
        "textures/block/stone.png".to_owned(),
        Rect::new(0.0, 0.0, 1.0, 1.0),
    );

    let mut chunk = Chunk::default();
    chunk.blocks[8][0][8] = BlockType::Stone;
    chunk.blocks[8][0][9] = BlockType::Stone;

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let meshes = GreedyChunkMesher.mesh(ChunkMeshInput {
        blocks: &blocks,
        block_texture_map: &texture_map,
        ao_brightness: AO_BRIGHTNESS,
    });

    for (_layer, mesh) in &meshes {
        let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("missing UV_0");
        };
        let max_x: f32 = uvs.iter().map(|[u, _v]| *u).fold(0.0_f32, f32::max);
        let max_y: f32 = uvs.iter().map(|[_u, v]| *v).fold(0.0_f32, f32::max);
        assert!(
            max_x > 1.0 || max_y > 1.0,
            "merged quads should have UV_0 > 1.0 in at least one axis, got max_x={max_x}, max_y={max_y}"
        );
    }
}

#[test]
fn greedy_uv1_uses_tile_offset() {
    let mut texture_map = test_texture_map();
    // Use a non-trivial tile offset to verify UV_1 is correctly encoded
    texture_map.0.insert(
        "textures/block/stone.png".to_owned(),
        Rect::new(0.1, 0.2, 0.15, 0.25),
    );

    let mut chunk = Chunk::default();
    chunk.blocks[8][0][8] = BlockType::Stone;
    chunk.blocks[8][0][9] = BlockType::Stone;

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let meshes = GreedyChunkMesher.mesh(ChunkMeshInput {
        blocks: &blocks,
        block_texture_map: &texture_map,
        ao_brightness: AO_BRIGHTNESS,
    });

    for (_layer, mesh) in &meshes {
        let Some(VertexAttributeValues::Float32x2(uv1s)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1)
        else {
            panic!("missing UV_1");
        };
        for &[u, v] in uv1s {
            assert!(
                (u - 0.1).abs() < 1e-6,
                "UV_1.x should be tile_offset.x=0.1, got {u}"
            );
            assert!(
                (v - 0.2).abs() < 1e-6,
                "UV_1.y should be tile_offset.y=0.2, got {v}"
            );
        }
    }
}

#[test]
fn greedy_uv1_is_set_on_all_vertices() {
    let mut texture_map = test_texture_map();
    texture_map.0.insert(
        "textures/block/stone.png".to_owned(),
        Rect::new(0.5, 0.25, 0.6, 0.35),
    );

    let mut chunk = Chunk::default();
    chunk.blocks[8][0][8] = BlockType::Stone;

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let meshes = GreedyChunkMesher.mesh(ChunkMeshInput {
        blocks: &blocks,
        block_texture_map: &texture_map,
        ao_brightness: AO_BRIGHTNESS,
    });

    for (_, mesh) in &meshes {
        let Some(VertexAttributeValues::Float32x2(uv1s)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1)
        else {
            panic!("missing UV_1");
        };
        for &[u, v] in uv1s {
            assert!(
                (u - 0.5).abs() < 1e-6,
                "UV_1.x should be tile_offset.x=0.5, got {u}"
            );
            assert!(
                (v - 0.25).abs() < 1e-6,
                "UV_1.y should be tile_offset.y=0.25, got {v}"
            );
        }
    }
}

#[test]
fn greedy_uv1_debug_all_values() {
    let mut texture_map = test_texture_map();
    texture_map.0.insert(
        "textures/block/stone.png".to_owned(),
        Rect::new(0.5, 0.25, 0.6, 0.35),
    );

    let mut chunk = Chunk::default();
    chunk.blocks[8][0][8] = BlockType::Stone;

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let meshes = GreedyChunkMesher.mesh(ChunkMeshInput {
        blocks: &blocks,
        block_texture_map: &texture_map,
        ao_brightness: AO_BRIGHTNESS,
    });

    for (layer, mesh) in &meshes {
        let Some(VertexAttributeValues::Float32x2(uv1s)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1)
        else {
            panic!("missing UV_1 in {layer:?}");
        };

        for &[u, v] in uv1s {
            assert!(
                (u - 0.5).abs() < 1e-6,
                "UV_1.x should be tile_offset.x=0.5, got {u}"
            );
            assert!(
                (v - 0.25).abs() < 1e-6,
                "UV_1.y should be tile_offset.y=0.25, got {v}"
            );
        }
    }
}

#[test]
fn greedy_vertex_count_less_than_direct_for_adjacent_blocks() {
    let mut texture_map = test_texture_map();
    texture_map.0.insert(
        "textures/block/stone.png".to_owned(),
        Rect::new(0.0, 0.0, 1.0, 1.0),
    );

    let mut chunk = Chunk::default();
    chunk.blocks[8][0][8] = BlockType::Stone;
    chunk.blocks[8][0][9] = BlockType::Stone;

    let blocks = ChunkMeshBlocks::from_chunk(&chunk);
    let input = ChunkMeshInput {
        blocks: &blocks,
        block_texture_map: &texture_map,
        ao_brightness: AO_BRIGHTNESS,
    };

    let direct_verts: usize = DirectChunkMesher
        .mesh(input)
        .iter()
        .map(
            |(_, m)| match m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
                VertexAttributeValues::Float32x3(v) => v.len(),
                _ => 0,
            },
        )
        .sum();

    let greedy_verts: usize = GreedyChunkMesher
        .mesh(input)
        .iter()
        .map(
            |(_, m)| match m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
                VertexAttributeValues::Float32x3(v) => v.len(),
                _ => 0,
            },
        )
        .sum();

    assert!(
        greedy_verts < direct_verts,
        "greedy ({greedy_verts}) should merge faces and have fewer vertices than direct ({direct_verts})"
    );
}

#[test]
fn partitioned_matches_greedy_for_all_test_chunks() {
    let texture_map = test_texture_map();

    for case in test_chunks() {
        let blocks = ChunkMeshBlocks::from_chunk(&case.chunk);
        let input = ChunkMeshInput {
            blocks: &blocks,
            block_texture_map: &texture_map,
            ao_brightness: AO_BRIGHTNESS,
        };

        assert_eq!(
            mesh_signature(GreedyChunkMesher.mesh(input)),
            mesh_signature(PartitionedGreedyChunkMesher.mesh(input)),
            "partitioned should match greedy for {}",
            case.name
        );
    }
}
