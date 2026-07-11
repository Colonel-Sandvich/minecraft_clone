//! Scalar reference meshing and the non-full-cube half of hybrid meshing.

use crate::block::{
    BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_TRANSLUCENT, BlockMaterialLayer, WATER_RENDER_ID,
};

use super::{
    super::{
        blocks::{ChunkMeshBlocks, DIRECTION_COUNT, DIRECTION_INDEX_OFFSETS, padded_chunk_index},
        face::PackedFace,
    },
    FacesByLayer, LayerMesh,
    ao::face_ao_key_from_indices,
    collect_layers, face_capacity_estimate,
    visibility::{
        block_mesh_flags, material_layer_index_from_flags, should_emit_face_from_flags,
        should_emit_translucent_face,
    },
    water::WaterFaceData,
};

pub(super) fn build_reference(blocks: &ChunkMeshBlocks) -> Vec<LayerMesh> {
    if blocks.can_skip_mesh() {
        return Vec::new();
    }

    let layer_capacity = face_capacity_estimate(blocks.center_rendered_blocks);
    let mut faces: FacesByLayer = std::array::from_fn(|_| Vec::with_capacity(layer_capacity));
    push_faces::<false>(blocks, &mut faces);
    collect_layers(faces)
}

pub(super) fn push_non_full_cube(blocks: &ChunkMeshBlocks, faces: &mut FacesByLayer) {
    push_faces::<true>(blocks, faces);
}

fn push_faces<const NON_FULL_CUBE_ONLY: bool>(blocks: &ChunkMeshBlocks, faces: &mut FacesByLayer) {
    for y in 0..crate::world::chunk::CHUNK_SIZE {
        for z in 0..crate::world::chunk::CHUNK_SIZE {
            let mut padded_index = padded_chunk_index(1, y + 1, z + 1);

            for x in 0..crate::world::chunk::CHUNK_SIZE {
                let render_id = unsafe { *blocks.blocks.get_unchecked(padded_index) };
                let flags = block_mesh_flags(render_id);

                if flags == 0 || (NON_FULL_CUBE_ONLY && flags & BLOCK_FLAG_FULL_CUBE != 0) {
                    padded_index += 1;
                    continue;
                }

                let is_water = render_id == WATER_RENDER_ID;
                let mut water_data = None;

                for (side_index, offset) in DIRECTION_INDEX_OFFSETS.iter().copied().enumerate() {
                    let neighbor_index = (padded_index as isize + offset) as usize;
                    let neighbor = unsafe { *blocks.blocks.get_unchecked(neighbor_index) };
                    let neighbor_flags = block_mesh_flags(neighbor);
                    let visible = if flags & BLOCK_FLAG_TRANSLUCENT != 0 {
                        should_emit_translucent_face(render_id, flags, neighbor, neighbor_flags)
                    } else {
                        should_emit_face_from_flags(render_id, flags, neighbor, neighbor_flags)
                    };

                    if !visible {
                        continue;
                    }

                    let ao_key = face_ao_key_from_indices(blocks, padded_index, side_index);
                    let face = PackedFace::new(
                        x as u32,
                        y as u32,
                        z as u32,
                        side_index as u32,
                        render_id as u32,
                        ao_key,
                    );
                    faces[material_layer_index_from_flags(flags)].push(if is_water {
                        water_data
                            .get_or_insert_with(|| WaterFaceData::from_cell(blocks, padded_index))
                            .apply(face, side_index)
                    } else {
                        face
                    });
                }

                padded_index += 1;
            }
        }
    }
}

const _: () = assert!(DIRECTION_COUNT == 6);
const _: () = assert!(BlockMaterialLayer::COUNT == 3);
