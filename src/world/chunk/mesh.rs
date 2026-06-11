use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::math::UVec3;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use itertools::Itertools;
use strum::IntoEnumIterator;

use crate::textures::{BlockStandardMaterials, TextureState};
use crate::{
    block::{
        BlockMaterialLayer, BlockTextureMap, BlockType, BlockUpdateMessage, FaceOcclusion,
        FaceSidedness, block_to_colour,
    },
    quad::{Direction, Quad, QuadGroups, get_indices, get_normals, get_positions, urect_to_uvs},
};

use super::{CHUNK_ISIZE, Chunk};

pub struct ChunkMeshPlugin;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkMaterialLayerMarker(BlockMaterialLayer);

#[derive(Debug, Default)]
pub struct LayeredQuadGroups {
    pub layers: [QuadGroups; BlockMaterialLayer::COUNT],
}

impl Plugin for ChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            update_mesh_simple.run_if(in_state(TextureState::Finished)),
        );
    }
}

fn update_mesh_simple(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    block_materials: Res<BlockStandardMaterials>,
    block_texture_map: Res<BlockTextureMap>,
    mut block_updates: MessageReader<BlockUpdateMessage>,
    added_chunks_q: Query<(&Chunk, Entity, Option<&Children>), Added<Chunk>>,
    chunks_q: Query<(&Chunk, Entity, Option<&Children>)>,
    mesh_children_q: Query<(Entity, &ChunkMaterialLayerMarker), With<Mesh3d>>,
    mut mesh_q: Query<&mut Mesh3d>,
) {
    for (chunk, chunk_entity, children) in added_chunks_q
        .iter()
        .chain(chunks_q.iter_many(block_updates.read().map(|u| u.chunk).unique()))
    {
        update_chunk_mesh_children(
            &mut commands,
            &mut meshes,
            &block_materials,
            &block_texture_map,
            &mesh_children_q,
            &mut mesh_q,
            chunk,
            chunk_entity,
            children,
        );
    }
}

fn update_chunk_mesh_children(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    block_materials: &BlockStandardMaterials,
    block_texture_map: &BlockTextureMap,
    mesh_children_q: &Query<(Entity, &ChunkMaterialLayerMarker), With<Mesh3d>>,
    mesh_q: &mut Query<&mut Mesh3d>,
    chunk: &Chunk,
    chunk_entity: Entity,
    children: Option<&Children>,
) {
    let existing_mesh_children = children
        .map(|children| {
            mesh_children_q
                .iter_many(children)
                .map(|(entity, marker)| (marker.0, entity))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut updated_layers = Vec::new();
    for (layer, mesh) in make_chunk_meshes(chunk, block_texture_map) {
        let mesh_handle = meshes.add(mesh);
        updated_layers.push(layer);

        if let Some(entity) = existing_mesh_children.get(&layer) {
            *mesh_q.get_mut(*entity).unwrap() = Mesh3d(mesh_handle);
            // Recompute Aabb after swapping mesh data.
            commands.entity(*entity).remove::<Aabb>();
            continue;
        }

        commands.spawn((
            ChildOf(chunk_entity),
            ChunkMaterialLayerMarker(layer),
            Mesh3d(mesh_handle),
            MeshMaterial3d(block_materials.get(layer)),
        ));
    }

    for (layer, entity) in existing_mesh_children {
        if updated_layers.contains(&layer) {
            continue;
        }

        commands.entity(entity).despawn();
    }
}

pub fn make_chunk_meshes(
    chunk: &Chunk,
    block_texture_map: &BlockTextureMap,
) -> Vec<(BlockMaterialLayer, Mesh)> {
    let quad_groups = make_layered_quad_groups(chunk, block_texture_map);
    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            make_mesh_from_quad_groups(&quad_groups.layers[layer.index()]).map(|mesh| (layer, mesh))
        })
        .collect()
}

pub fn make_layered_quad_groups(
    chunk: &Chunk,
    block_texture_map: &BlockTextureMap,
) -> LayeredQuadGroups {
    let mut buffer = LayeredQuadGroups::default();

    for x in 0..CHUNK_ISIZE {
        for y in 0..CHUNK_ISIZE {
            for z in 0..CHUNK_ISIZE {
                let Some(block) = chunk.get_i(x, y, z) else {
                    continue;
                };

                let Some(profile) = block.render_profile() else {
                    continue;
                };

                let neighbours = [
                    chunk.get_i(x - 1, y, z),
                    chunk.get_i(x + 1, y, z),
                    chunk.get_i(x, y - 1, z),
                    chunk.get_i(x, y + 1, z),
                    chunk.get_i(x, y, z - 1),
                    chunk.get_i(x, y, z + 1),
                ];

                for (neighbour, side) in neighbours.into_iter().zip(Direction::iter()) {
                    let neighbour = neighbour.unwrap_or(BlockType::Air);

                    if !should_emit_face(block, neighbour, side) {
                        continue;
                    }

                    buffer.layers[profile.material_layer().index()].groups[side as usize].push(
                        Quad {
                            voxel: UVec3::from_slice(&[x, y, z].map(|u| u.try_into().unwrap())),
                            uv: block_texture_map.block_to_mesh(block, side),
                            color: block_to_colour(block, side),
                        },
                    );
                }
            }
        }
    }

    buffer
}

fn should_emit_face(block: BlockType, neighbour: BlockType, side: Direction) -> bool {
    let Some(block_profile) = block.render_profile() else {
        return false;
    };
    let Some(neighbour_profile) = neighbour.render_profile() else {
        return true;
    };

    if neighbour_profile.occlusion == FaceOcclusion::FullCube {
        return false;
    }

    if block == neighbour
        && block_profile.occlusion == FaceOcclusion::None
        && neighbour_profile.occlusion == FaceOcclusion::None
    {
        return block_profile.sidedness == FaceSidedness::Double && is_positive_side(side);
    }

    true
}

fn is_positive_side(side: Direction) -> bool {
    matches!(side, Direction::Right | Direction::Up | Direction::Backward)
}

pub fn make_mesh_from_quad_groups(quad_groups: &QuadGroups) -> Option<Mesh> {
    let len: usize = quad_groups.groups.iter().map(|g| g.len()).sum();

    if len == 0 {
        return None;
    }

    let num_indices = len * 6;
    let num_vertices = len * 4;

    let mut indices = Vec::with_capacity(num_indices);
    let mut positions = Vec::with_capacity(num_vertices);
    let mut normals = Vec::with_capacity(num_vertices);
    let mut uvs = Vec::with_capacity(num_vertices);
    let mut colours = Vec::with_capacity(num_vertices);

    for (quads, side) in quad_groups.groups.iter().zip(Direction::iter()) {
        for quad in quads.iter() {
            indices.extend_from_slice(&get_indices(positions.len() as u32));
            positions.extend_from_slice(&get_positions(quad, &side, 1.0));
            normals.extend_from_slice(&get_normals(side.into()));
            uvs.extend_from_slice(&urect_to_uvs(&quad.uv));
            colours.extend_from_slice(&[quad.color; 4]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );

    mesh.insert_indices(Indices::U32(indices));

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);

    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{math::Rect, platform::collections::HashMap};
    use strum::IntoEnumIterator;

    use crate::block::block_and_side_to_texture_path;

    fn test_texture_map() -> BlockTextureMap {
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

        BlockTextureMap(paths)
    }

    fn quad_count(groups: &QuadGroups) -> usize {
        groups.groups.iter().map(Vec::len).sum()
    }

    #[test]
    fn adjacent_leaves_emit_one_shared_double_sided_face() {
        let texture_map = test_texture_map();
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::OakLeaves;
        chunk.blocks[1][0][0] = BlockType::OakLeaves;

        let groups = make_layered_quad_groups(&chunk, &texture_map);
        let leaf_groups = &groups.layers[BlockMaterialLayer::CutoutDoubleSided.index()];

        assert_eq!(quad_count(leaf_groups), 11);
        assert_eq!(
            groups.layers[BlockMaterialLayer::Opaque.index()]
                .groups
                .iter()
                .map(Vec::len)
                .sum::<usize>(),
            0
        );
        assert_eq!(leaf_groups.groups[Direction::Right as usize].len(), 2);
        assert_eq!(leaf_groups.groups[Direction::Left as usize].len(), 1);
    }

    #[test]
    fn leaves_do_not_occlude_opaque_faces() {
        let texture_map = test_texture_map();
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Stone;
        chunk.blocks[1][0][0] = BlockType::OakLeaves;

        let groups = make_layered_quad_groups(&chunk, &texture_map);
        let opaque_groups = &groups.layers[BlockMaterialLayer::Opaque.index()];
        let leaf_groups = &groups.layers[BlockMaterialLayer::CutoutDoubleSided.index()];

        assert_eq!(quad_count(opaque_groups), 6);
        assert_eq!(quad_count(leaf_groups), 5);
        assert_eq!(opaque_groups.groups[Direction::Right as usize].len(), 1);
        assert_eq!(leaf_groups.groups[Direction::Left as usize].len(), 0);
    }

    #[test]
    fn chunk_meshes_are_split_by_render_layer() {
        let texture_map = test_texture_map();
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Stone;
        chunk.blocks[3][0][0] = BlockType::OakLeaves;

        let layers = make_chunk_meshes(&chunk, &texture_map)
            .into_iter()
            .map(|(layer, _)| layer)
            .collect::<Vec<_>>();

        assert_eq!(
            layers,
            vec![
                BlockMaterialLayer::Opaque,
                BlockMaterialLayer::CutoutDoubleSided
            ]
        );
    }
}
