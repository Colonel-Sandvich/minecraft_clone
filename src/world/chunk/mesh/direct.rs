use crate::block::BlockMaterialLayer;

use super::{
    BLOCK_IS_RENDERED, BLOCK_MATERIAL_LAYER_INDEX, BlockMeshTables, CHUNK_SIZE, ChunkLayerMeshes,
    ChunkMeshInput, ChunkMesher, DIRECTION_COUNT, DIRECTION_INDEX_OFFSETS, MeshBufferBuilder,
    block_mesh_index, face_ao_from_indices, padded_chunk_index, should_emit_face_from_indices,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct DirectChunkMesher;

impl ChunkMesher for DirectChunkMesher {
    fn name(&self) -> &'static str {
        "direct"
    }

    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
        if input.blocks.can_skip_mesh() {
            return Vec::new();
        }

        make_direct_chunk_meshes(input)
    }
}

fn make_direct_chunk_meshes(input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
    let tables = BlockMeshTables::from_texture_map(input.block_texture_map);
    let face_counts = count_direct_faces(input, tables);
    let mut builders: [MeshBufferBuilder; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|i| MeshBufferBuilder::with_face_capacity(face_counts[i]));

    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let mut padded_index = padded_chunk_index(1, y + 1, z + 1);

            for x in 0..CHUNK_SIZE {
                let block = input.blocks.blocks[padded_index];
                let block_index = block_mesh_index(block);

                if !BLOCK_IS_RENDERED[block_index] {
                    padded_index += 1;
                    continue;
                }

                for side_index in 0..DIRECTION_COUNT {
                    let neighbor = input.blocks.blocks
                        [(padded_index as isize + DIRECTION_INDEX_OFFSETS[side_index]) as usize];
                    let neighbor_index = block_mesh_index(neighbor);

                    if !should_emit_face_from_indices(block_index, neighbor_index, side_index) {
                        continue;
                    }

                    let ao = face_ao_from_indices(input.blocks, padded_index, side_index);
                    builders[BLOCK_MATERIAL_LAYER_INDEX[block_index]].push_face(
                        x,
                        y,
                        z,
                        side_index,
                        tables.uvs[block_index][side_index],
                        tables.colors[block_index][side_index],
                        ao,
                        input.ao_brightness,
                    );
                }

                padded_index += 1;
            }
        }
    }

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            std::mem::take(&mut builders[layer.index()])
                .into_mesh()
                .map(|mesh| (layer, mesh))
        })
        .collect()
}

fn count_direct_faces(
    input: ChunkMeshInput<'_>,
    _tables: BlockMeshTables,
) -> [usize; BlockMaterialLayer::COUNT] {
    let mut counts = [0; BlockMaterialLayer::COUNT];

    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let mut padded_index = padded_chunk_index(1, y + 1, z + 1);

            for _x in 0..CHUNK_SIZE {
                let block = input.blocks.blocks[padded_index];
                let block_index = block_mesh_index(block);

                if !BLOCK_IS_RENDERED[block_index] {
                    padded_index += 1;
                    continue;
                }

                for side_index in 0..DIRECTION_COUNT {
                    let neighbor = input.blocks.blocks
                        [(padded_index as isize + DIRECTION_INDEX_OFFSETS[side_index]) as usize];
                    let neighbor_index = block_mesh_index(neighbor);

                    counts[BLOCK_MATERIAL_LAYER_INDEX[block_index]] +=
                        should_emit_face_from_indices(block_index, neighbor_index, side_index)
                            as usize;
                }

                padded_index += 1;
            }
        }
    }

    counts
}
