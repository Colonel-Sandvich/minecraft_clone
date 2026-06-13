use crate::block::BlockMaterialLayer;

use super::{
    BlockMeshTables, CHUNK_SIZE, ChunkLayerMeshes, ChunkMeshBlocks, ChunkMeshInput, ChunkMesher,
    DIRECTION_INDEX_OFFSETS, MeshBufferBuilder, face_ao_from_indices,
    padded_chunk_index,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct FullCubeShellChunkMesher;

impl ChunkMesher for FullCubeShellChunkMesher {
    fn name(&self) -> &'static str {
        "full_cube_shell"
    }

    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
        if input.blocks.can_skip_mesh() {
            return Vec::new();
        }

        make_full_cube_shell_chunk_meshes(input)
    }
}

fn make_full_cube_shell_chunk_meshes(input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
    let tables = BlockMeshTables::from_texture_map(input.block_texture_map);
    let face_counts = count_full_cube_shell_faces(input.blocks, tables);
    let mut builders: [MeshBufferBuilder; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|i| MeshBufferBuilder::with_face_capacity(face_counts[i]));

    emit_full_cube_shell_faces(input, tables, &mut builders);

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            std::mem::take(&mut builders[layer.index()])
                .into_mesh()
                .map(|mesh| (layer, mesh))
        })
        .collect()
}

fn count_full_cube_shell_faces(
    blocks: &ChunkMeshBlocks,
    _tables: BlockMeshTables,
) -> [usize; BlockMaterialLayer::COUNT] {
    let mut counts = [0; BlockMaterialLayer::COUNT];

    for_full_cube_shell_face(blocks, |_x, _y, _z, padded_index, side_index| {
        let neighbor =
            blocks.blocks[(padded_index as isize + DIRECTION_INDEX_OFFSETS[side_index]) as usize];
        if neighbor.is_full_cube() {
            return;
        }

        let block = blocks.blocks[padded_index];
        counts[block.material_layer_index()] += 1;
    });

    counts
}

fn emit_full_cube_shell_faces(
    input: ChunkMeshInput<'_>,
    tables: BlockMeshTables,
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
) {
    for_full_cube_shell_face(input.blocks, |x, y, z, padded_index, side_index| {
        let neighbor = input.blocks.blocks
            [(padded_index as isize + DIRECTION_INDEX_OFFSETS[side_index]) as usize];
        if neighbor.is_full_cube() {
            return;
        }

        let block = input.blocks.blocks[padded_index];
        let block_index = block as usize;
        let ao = face_ao_from_indices(input.blocks, padded_index, side_index);

        builders[block.material_layer_index()].push_face(
            x,
            y,
            z,
            side_index,
            tables.uvs[block_index][side_index],
            tables.colors[block_index][side_index],
            ao,
            input.ao_brightness,
        );
    });
}

fn for_full_cube_shell_face(
    blocks: &ChunkMeshBlocks,
    mut visit: impl FnMut(usize, usize, usize, usize, usize),
) {
    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            visit_shell_face(blocks, 0, y, z, 0, &mut visit);
            visit_shell_face(blocks, CHUNK_SIZE - 1, y, z, 1, &mut visit);
        }
    }

    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            visit_shell_face(blocks, x, 0, z, 2, &mut visit);
            visit_shell_face(blocks, x, CHUNK_SIZE - 1, z, 3, &mut visit);
        }
    }

    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            visit_shell_face(blocks, x, y, 0, 4, &mut visit);
            visit_shell_face(blocks, x, y, CHUNK_SIZE - 1, 5, &mut visit);
        }
    }
}

#[inline(always)]
fn visit_shell_face(
    blocks: &ChunkMeshBlocks,
    x: usize,
    y: usize,
    z: usize,
    side_index: usize,
    visit: &mut impl FnMut(usize, usize, usize, usize, usize),
) {
    let padded_index = padded_chunk_index(x + 1, y + 1, z + 1);
    debug_assert!(blocks.blocks[padded_index].is_full_cube());
    visit(x, y, z, padded_index, side_index);
}
