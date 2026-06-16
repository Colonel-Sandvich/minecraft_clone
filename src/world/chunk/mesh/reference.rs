use bevy::math::IVec3;
use strum::IntoEnumIterator;

use crate::block::{
    BlockMaterialLayer, BlockTextureMap, BlockType, FaceOcclusion, block_to_colour,
};
use crate::quad::{Direction, Quad};
use crate::world::chunk::{CHUNK_ISIZE, Chunk};

use super::{
    ChunkLayerMeshes, ChunkMeshBlocks, ChunkMeshInput, ChunkMesher, LayeredQuadGroups,
    make_mesh_from_quad_groups_with_ao_brightness,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct ReferenceChunkMesher;

impl ChunkMesher for ReferenceChunkMesher {
    fn name(&self) -> &'static str {
        "reference"
    }

    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
        if input.blocks.can_skip_mesh() {
            return Vec::new();
        }

        make_chunk_meshes_from_blocks_with_ao_brightness(
            input.blocks,
            input.block_texture_map,
            input.ao_brightness,
        )
    }
}

fn make_chunk_meshes_from_blocks_with_ao_brightness(
    blocks: &(impl BlockSampler + ?Sized),
    block_texture_map: &BlockTextureMap,
    ao_brightness: [f32; 4],
) -> ChunkLayerMeshes {
    let quad_groups = make_layered_quad_groups_from_blocks(blocks, block_texture_map);
    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            make_mesh_from_quad_groups_with_ao_brightness(
                &quad_groups.layers[layer.index()],
                ao_brightness,
            )
            .map(|mesh| (layer, mesh))
        })
        .collect()
}

pub(crate) fn make_layered_quad_groups_from_blocks(
    blocks: &(impl BlockSampler + ?Sized),
    block_texture_map: &BlockTextureMap,
) -> LayeredQuadGroups {
    let mut buffer = LayeredQuadGroups::default();

    for x in 0..CHUNK_ISIZE {
        for y in 0..CHUNK_ISIZE {
            for z in 0..CHUNK_ISIZE {
                let block = blocks.get_block(x, y, z);

                let Some(profile) = block.render_profile() else {
                    continue;
                };

                let neighbors = [
                    blocks.get_block(x - 1, y, z),
                    blocks.get_block(x + 1, y, z),
                    blocks.get_block(x, y - 1, z),
                    blocks.get_block(x, y + 1, z),
                    blocks.get_block(x, y, z - 1),
                    blocks.get_block(x, y, z + 1),
                ];

                for (neighbor, side) in neighbors.into_iter().zip(Direction::iter()) {
                    if !should_emit_face(block, neighbor, side) {
                        continue;
                    }

                    let voxel =
                        bevy::math::UVec3::from_slice(&[x, y, z].map(|u| u.try_into().unwrap()));

                    buffer.layers[profile.material_layer().index()].groups[side as usize].push(
                        Quad {
                            voxel,
                            uv: block_texture_map.block_to_mesh(block, side),
                            color: block_to_colour(block, side),
                            ao: face_ao(blocks, IVec3::new(x, y, z), side),
                            block_type: block,
                        },
                    );
                }
            }
        }
    }

    buffer
}

pub(crate) fn face_ao(
    blocks: &(impl BlockSampler + ?Sized),
    voxel: IVec3,
    side: Direction,
) -> [u8; 4] {
    let normal = direction_offset(side);
    let tangent_axes = tangent_axes(side);

    crate::quad::get_vertex_offsets(side).map(|vertex_offset| {
        let side1 = sample_axis_offset(vertex_offset, tangent_axes[0]);
        let side2 = sample_axis_offset(vertex_offset, tangent_axes[1]);

        vertex_ao(
            ao_occludes(blocks, voxel + normal + side1),
            ao_occludes(blocks, voxel + normal + side2),
            ao_occludes(blocks, voxel + normal + side1 + side2),
        )
    })
}

fn should_emit_face(block: BlockType, neighbor: BlockType, _side: Direction) -> bool {
    let Some(block_profile) = block.render_profile() else {
        return false;
    };
    let Some(neighbor_profile) = neighbor.render_profile() else {
        return true;
    };

    if neighbor_profile.occlusion == FaceOcclusion::FullCube {
        return false;
    }

    if block == neighbor
        && block_profile.occlusion == FaceOcclusion::None
        && neighbor_profile.occlusion == FaceOcclusion::None
    {
        return block_is_leaf(block);
    }

    true
}

fn block_is_leaf(block: BlockType) -> bool {
    matches!(block, BlockType::OakLeaves)
}

pub(crate) fn vertex_ao(side1: bool, side2: bool, corner: bool) -> u8 {
    if side1 && side2 {
        0
    } else {
        3 - side1 as u8 - side2 as u8 - corner as u8
    }
}

fn ao_occludes(blocks: &(impl BlockSampler + ?Sized), pos: IVec3) -> bool {
    block_occludes_ambient_light(blocks.get_block(pos.x, pos.y, pos.z))
}

fn block_occludes_ambient_light(block: BlockType) -> bool {
    block
        .render_profile()
        .is_some_and(|profile| profile.occlusion == FaceOcclusion::FullCube)
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
        _ => unreachable!("invalid axis"),
    }
}

fn axis_component(offset: IVec3, axis: usize) -> i32 {
    match axis {
        0 => offset.x,
        1 => offset.y,
        2 => offset.z,
        _ => unreachable!("invalid axis"),
    }
}

pub(crate) trait BlockSampler: Sync {
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockType;
}

impl BlockSampler for Chunk {
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockType {
        self.get_i(x, y, z).unwrap_or(BlockType::Air)
    }
}

impl BlockSampler for ChunkMeshBlocks {
    fn get_block(&self, x: i32, y: i32, z: i32) -> BlockType {
        if !is_in_padded_chunk(x) || !is_in_padded_chunk(y) || !is_in_padded_chunk(z) {
            return BlockType::Air;
        }

        let x = (x + 1) as usize;
        let y = (y + 1) as usize;
        let z = (z + 1) as usize;
        self.blocks[super::padded_chunk_index(x, y, z)]
    }
}

fn is_in_padded_chunk(value: i32) -> bool {
    (-1..=CHUNK_ISIZE).contains(&value)
}
