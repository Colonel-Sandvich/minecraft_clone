use bevy::{
    math::UVec3,
    render::{
        mesh::{Indices, Mesh, PrimitiveTopology},
        render_asset::RenderAssetUsages,
    },
};
use strum::IntoEnumIterator;

use crate::{
    block::{block_to_colour, BlockTextureMap, BlockType},
    chunk::{Chunk, CHUNK_ISIZE},
    quad::{get_indices, get_normals, get_positions, rect_to_uvs, Direction, Quad, QuadGroups},
};

pub fn make_quad_groups(chunk: &Chunk, block_texture_map: &BlockTextureMap) -> QuadGroups {
    let mut buffer = QuadGroups::default();

    for x in 0..CHUNK_ISIZE {
        for y in 0..CHUNK_ISIZE {
            for z in 0..CHUNK_ISIZE {
                let Some(block) = chunk.get(x, y, z) else {
                    continue;
                };

                if !block.visible() {
                    continue;
                }

                let neighbours = [
                    chunk.get(x - 1, y, z),
                    chunk.get(x + 1, y, z),
                    chunk.get(x, y - 1, z),
                    chunk.get(x, y + 1, z),
                    chunk.get(x, y, z - 1),
                    chunk.get(x, y, z + 1),
                ];

                for (neighbour, side) in neighbours.into_iter().zip(Direction::iter()) {
                    let neighbour = neighbour.unwrap_or(BlockType::Air);

                    use crate::block::Visibility::*;

                    if match (block.visibility(), neighbour.visibility()) {
                        (Opaque, Empty) | (Opaque, Translucent) | (Translucent, Empty) => false,
                        // (Transparent, Transparent) => block == neighbour,
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

pub fn make_mesh(quad_groups: &QuadGroups) -> Mesh {
    let mut indices = Vec::new();
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colours = Vec::new();

    for (quads, side) in quad_groups.groups.iter().zip(Direction::iter()) {
        for quad in quads.iter() {
            indices.extend_from_slice(&get_indices(positions.len() as u32));
            positions.extend_from_slice(&get_positions(quad, &side, 1.0));
            normals.extend_from_slice(&get_normals(side.into()));
            uvs.extend_from_slice(&rect_to_uvs(&quad.uv));
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

    mesh
}
