use crate::block::BlockMaterialLayer;

use super::{
    BlockMeshTables, CHUNK_SIZE, CHUNK_VOLUME, ChunkLayerMeshes, ChunkMeshInput, ChunkMesher,
    DIRECTION_COUNT, DIRECTION_INDEX_OFFSETS, MeshBufferBuilder,
    face_ao_from_indices, padded_chunk_index, should_emit_face_from_indices,
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
    let hint = (input.blocks.center_rendered_blocks as usize * 6).min(CHUNK_VOLUME * 6);
    let mut builders: [MeshBufferBuilder; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|_| MeshBufferBuilder::with_face_capacity(hint));

    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let mut padded_index = padded_chunk_index(1, y + 1, z + 1);

            for x in 0..CHUNK_SIZE {
                let block = input.blocks.blocks[padded_index];

                if !block.is_rendered() {
                    padded_index += 1;
                    continue;
                }

                let block_index = block as usize;

                for side_index in 0..DIRECTION_COUNT {
                    let neighbor = input.blocks.blocks
                        [(padded_index as isize + DIRECTION_INDEX_OFFSETS[side_index]) as usize];

                    if !should_emit_face_from_indices(block, neighbor) {
                        continue;
                    }

                    let ao = face_ao_from_indices(input.blocks, padded_index, side_index);
                    let layer_idx = block.material_layer_index();
                    builders[layer_idx].push_face(
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
