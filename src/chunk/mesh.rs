use bevy::prelude::*;
use bevy::render::primitives::Aabb;
use bevy::{
    math::UVec3,
    render::{
        mesh::{Indices, Mesh, PrimitiveTopology},
        render_asset::RenderAssetUsages,
    },
};
use itertools::Itertools;
use strum::IntoEnumIterator;

use crate::block::BlockUpdateEvent;
use crate::textures::{BlockStandardMaterial, TextureState};
use crate::{
    block::{BlockTextureMap, BlockType, block_to_colour},
    chunk::{CHUNK_ISIZE, Chunk},
    quad::{Direction, Quad, QuadGroups, get_indices, get_normals, get_positions, urect_to_uvs},
};

pub struct ChunkMeshPlugin;

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
    block_material: Res<BlockStandardMaterial>,
    block_texture_map: Res<BlockTextureMap>,
    mut block_updates: EventReader<BlockUpdateEvent>,
    added_chunks_q: Query<(&Chunk, Entity, Option<&Children>), Added<Chunk>>,
    chunks_q: Query<(&Chunk, Entity, Option<&Children>)>,
    mut mesh_q: Query<(Entity, &mut Mesh3d)>,
) {
    for (chunk, chunk_entity, children) in added_chunks_q
        .iter()
        .chain(chunks_q.iter_many(block_updates.read().map(|u| u.chunk).unique()))
    {
        let Some(mesh) = make_mesh_simple(chunk, &block_texture_map) else {
            if let Some(children) = children {
                let mut meshes = mesh_q.iter_many(children);

                if let Some((pbr_entity, _)) = meshes.fetch_next() {
                    commands.get_entity(pbr_entity).unwrap().despawn();
                };

                assert_eq!(None, meshes.fetch_next());
            }
            continue;
        };

        let mesh_handle = meshes.add(mesh);

        if let Some(children) = children {
            let mut meshes = mesh_q.iter_many_mut(children);

            if let Some((pbr_entity, mut old_mesh)) = meshes.fetch_next() {
                *old_mesh = Mesh3d(mesh_handle);
                // Recompute Aabb
                commands.get_entity(pbr_entity).unwrap().remove::<Aabb>();
                continue;
            };
        };

        commands.spawn((
            ChildOf(chunk_entity),
            Mesh3d(mesh_handle),
            MeshMaterial3d(block_material.clone()),
        ));
    }
}

pub fn make_mesh_simple(chunk: &Chunk, block_texture_map: &BlockTextureMap) -> Option<Mesh> {
    make_mesh_from_quad_groups(&make_quad_groups(chunk, block_texture_map))
}

pub fn make_quad_groups(chunk: &Chunk, block_texture_map: &BlockTextureMap) -> QuadGroups {
    let mut buffer = QuadGroups::default();

    for x in 0..CHUNK_ISIZE {
        for y in 0..CHUNK_ISIZE {
            for z in 0..CHUNK_ISIZE {
                let Some(block) = chunk.get_i(x, y, z) else {
                    continue;
                };

                if !block.is_visible() {
                    continue;
                }

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

                    use crate::block::Visibility::*;

                    if match (block.visibility(), neighbour.visibility()) {
                        (Opaque, Empty) | (Opaque, Translucent) | (Translucent, Empty) => false,
                        (Translucent, Translucent) => block == neighbour,
                        (_, _) => true,
                    } {
                        continue;
                    }

                    buffer.groups[side as usize].push(Quad {
                        voxel: UVec3::from_slice(&[x, y, z].map(|u| u.try_into().unwrap())),
                        uv: block_texture_map.block_to_mesh(block, side),
                        color: block_to_colour(block, side),
                    });
                }
            }
        }
    }

    buffer
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
