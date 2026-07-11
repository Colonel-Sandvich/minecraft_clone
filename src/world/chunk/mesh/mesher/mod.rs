//! Chunk meshing algorithms.
//!
//! The production entry point uses binary face masks for full cubes and a
//! scalar pass for shaped/translucent blocks. Algorithm-specific entry points
//! remain public for benchmarks and equivalence tests, not for game systems.

mod ao;
mod binary;
mod scalar;
mod visibility;
mod water;

use crate::block::BlockMaterialLayer;

use super::{blocks::ChunkMeshBlocks, face::PackedFace};

type FacesByLayer = [Vec<PackedFace>; BlockMaterialLayer::COUNT];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerMesh {
    pub material_layer: BlockMaterialLayer,
    pub faces: Vec<PackedFace>,
}

/// Build all visible terrain faces using the production hybrid mesher.
pub fn build(blocks: &ChunkMeshBlocks) -> Vec<LayerMesh> {
    if blocks.can_skip_mesh() {
        return Vec::new();
    }

    if !blocks.has_non_full_cube_rendered() {
        return binary::build_binary(blocks);
    }

    let mut faces: FacesByLayer = std::array::from_fn(|_| Vec::new());
    faces[BlockMaterialLayer::Opaque.index()] =
        Vec::with_capacity(face_capacity_estimate(blocks.center_full_cube_blocks));

    let non_full_cube_cells = blocks
        .center_rendered_blocks
        .saturating_sub(blocks.center_full_cube_blocks);
    let non_full_cube_capacity = face_capacity_estimate(non_full_cube_cells);
    faces[BlockMaterialLayer::Cutout.index()] = Vec::with_capacity(non_full_cube_capacity);
    faces[BlockMaterialLayer::Translucent.index()] = Vec::with_capacity(non_full_cube_capacity);

    binary::push_binary_faces(blocks, &mut faces[BlockMaterialLayer::Opaque.index()]);
    scalar::push_non_full_cube(blocks, &mut faces);
    collect_layers(faces)
}

/// Scalar reference implementation used to verify the optimized mesher.
#[doc(hidden)]
pub fn build_reference(blocks: &ChunkMeshBlocks) -> Vec<LayerMesh> {
    scalar::build_reference(blocks)
}

/// Binary full-cube-only implementation used by focused benchmarks.
#[doc(hidden)]
pub fn build_binary(blocks: &ChunkMeshBlocks) -> Vec<LayerMesh> {
    binary::build_binary(blocks)
}

/// Lower-bound binary face-mask benchmark with packed-face work removed.
#[doc(hidden)]
pub fn benchmark_binary_floor(blocks: &ChunkMeshBlocks) -> usize {
    binary::benchmark_binary_floor(blocks)
}

/// Worst-case sparse-cell estimate, capped at the exterior of a dense chunk.
pub(super) fn face_capacity_estimate(rendered_cells: u16) -> usize {
    (usize::from(rendered_cells) * super::blocks::DIRECTION_COUNT).min(
        super::blocks::DIRECTION_COUNT
            * crate::world::chunk::CHUNK_SIZE
            * crate::world::chunk::CHUNK_SIZE,
    )
}

fn collect_layers(mut faces: FacesByLayer) -> Vec<LayerMesh> {
    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|material_layer| {
            let faces = std::mem::take(&mut faces[material_layer.index()]);
            (!faces.is_empty()).then_some(LayerMesh {
                material_layer,
                faces,
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) use ao::face_ao_from_indices;
#[cfg(test)]
pub(crate) use water::{water_below_pair, water_corner_heights};
