use crate::block::BlockMaterialLayer;

use super::{
    AO_SAMPLE_INDEX_OFFSETS, BlockMeshTables, BlockType, CHUNK_SIZE, ChunkLayerMeshes,
    ChunkMeshInput, ChunkMesher, DIRECTION_INDEX_OFFSETS, MeshBufferBuilder, PADDED_CHUNK_SIZE,
    PADDED_CHUNK_VOLUME, VERTEX_AO, padded_chunk_index, should_emit_face_from_indices,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct HybridChunkMesher;

impl ChunkMesher for HybridChunkMesher {
    fn name(&self) -> &'static str {
        "hybrid"
    }

    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
        if input.blocks.can_skip_mesh() {
            return Vec::new();
        }

        make_hybrid_chunk_meshes(input)
    }
}

const PLANE_U64S: usize = 6;
type BitPlane = [u64; PLANE_U64S];

fn plane_pack_index(a: usize, b: usize) -> usize {
    a * PADDED_CHUNK_SIZE + b
}

fn plane_set(plane: &mut BitPlane, idx: usize) {
    plane[idx / 64] |= 1 << (idx % 64);
}

fn bitwise_and_not(a: &BitPlane, b: &BitPlane) -> BitPlane {
    let mut r = [0u64; PLANE_U64S];
    for i in 0..PLANE_U64S {
        r[i] = a[i] & !b[i];
    }
    r
}

struct AxisMasks {
    full_cube: [BitPlane; PADDED_CHUNK_SIZE],
}

struct HybridData {
    masks: [AxisMasks; 3],
    transparent_count: usize,
}

fn build_bitmasks(blocks: &[BlockType; PADDED_CHUNK_VOLUME]) -> HybridData {
    let mut masks = std::array::from_fn(|_| AxisMasks {
        full_cube: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
    });
    let mut transparent_count = 0;

    for py in 0..PADDED_CHUNK_SIZE {
        for pz in 0..PADDED_CHUNK_SIZE {
            let mut pi = padded_chunk_index(0, py, pz);
            for px in 0..PADDED_CHUNK_SIZE {
                let block = blocks[pi];

                if block.is_full_cube() {
                    let yz = plane_pack_index(py, pz);
                    plane_set(&mut masks[0].full_cube[px], yz);

                    let xz = plane_pack_index(px, pz);
                    plane_set(&mut masks[1].full_cube[py], xz);

                    let xy = plane_pack_index(px, py);
                    plane_set(&mut masks[2].full_cube[pz], xy);
                } else if block.is_rendered() {
                    transparent_count += 1;
                }

                pi += 1;
            }
        }
    }

    HybridData {
        masks,
        transparent_count,
    }
}

#[inline(always)]
fn block_occludes(blocks: &[BlockType; PADDED_CHUNK_VOLUME], pi: usize, offset: isize) -> bool {
    blocks[(pi as isize + offset) as usize].is_full_cube()
}

#[inline(always)]
fn compute_ao(blocks: &[BlockType; PADDED_CHUNK_VOLUME], pi: usize, dir: usize) -> [u8; 4] {
    AO_SAMPLE_INDEX_OFFSETS[dir].map(|o| {
        let s1 = block_occludes(blocks, pi, o[0]);
        let s2 = block_occludes(blocks, pi, o[1]);
        let co = block_occludes(blocks, pi, o[2]);
        VERTEX_AO[s1 as usize | ((s2 as usize) << 1) | ((co as usize) << 2)]
    })
}

fn count_faces(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    data: &HybridData,
) -> [usize; BlockMaterialLayer::COUNT] {
    let mut counts = [0; BlockMaterialLayer::COUNT];

    for axis in 0..3usize {
        let axis_masks = &data.masks[axis];

        for c in 1..=CHUNK_SIZE {
            let emit_first =
                bitwise_and_not(&axis_masks.full_cube[c], &axis_masks.full_cube[c - 1]);
            let emit_second =
                bitwise_and_not(&axis_masks.full_cube[c], &axis_masks.full_cube[c + 1]);

            for wi in 0..PLANE_U64S {
                counts[0] += emit_first[wi].count_ones() as usize;
                counts[0] += emit_second[wi].count_ones() as usize;
            }
        }
    }

    if data.transparent_count > 0 {
        for px in 1..=CHUNK_SIZE {
            for py in 1..=CHUNK_SIZE {
                for pz in 1..=CHUNK_SIZE {
                    let pi = padded_chunk_index(px, py, pz);
                    let block = blocks[pi];

                    if block.is_full_cube() || !block.is_rendered() {
                        continue;
                    }

                    for dir in 0..6usize {
                        let neighbor =
                            blocks[(pi as isize + DIRECTION_INDEX_OFFSETS[dir]) as usize];
                        if should_emit_face_from_indices(block, neighbor) {
                            counts[block.material_layer_index()] += 1;
                        }
                    }
                }
            }
        }
    }

    counts
}

fn emit_faces(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    data: &HybridData,
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
) {
    for axis in 0..3usize {
        let axis_masks = &data.masks[axis];
        let first_dir = axis * 2;
        let second_dir = axis * 2 + 1;

        for c in 1..=CHUNK_SIZE {
            let emit_first =
                bitwise_and_not(&axis_masks.full_cube[c], &axis_masks.full_cube[c - 1]);
            let emit_second =
                bitwise_and_not(&axis_masks.full_cube[c], &axis_masks.full_cube[c + 1]);

            emit_plane_opaque(
                &emit_first,
                blocks,
                tables,
                ao_brightness,
                builders,
                c,
                axis,
                first_dir,
            );
            emit_plane_opaque(
                &emit_second,
                blocks,
                tables,
                ao_brightness,
                builders,
                c,
                axis,
                second_dir,
            );
        }
    }

    if data.transparent_count > 0 {
        for px in 1..=CHUNK_SIZE {
            for py in 1..=CHUNK_SIZE {
                for pz in 1..=CHUNK_SIZE {
                    let pi = padded_chunk_index(px, py, pz);
                    let block = blocks[pi];

                    if block.is_full_cube() || !block.is_rendered() {
                        continue;
                    }

                    let wx = px - 1;
                    let wy = py - 1;
                    let wz = pz - 1;
                    let bi = block as usize;

                    for dir in 0..6usize {
                        let neighbor =
                            blocks[(pi as isize + DIRECTION_INDEX_OFFSETS[dir]) as usize];
                        if should_emit_face_from_indices(block, neighbor) {
                            let ao = compute_ao(blocks, pi, dir);
                            builders[block.material_layer_index()].push_face(
                                wx,
                                wy,
                                wz,
                                dir,
                                tables.uvs[bi][dir],
                                tables.colors[bi][dir],
                                ao,
                                ao_brightness,
                            );
                        }
                    }
                }
            }
        }
    }
}

fn emit_plane_opaque(
    emit: &BitPlane,
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
    c: usize,
    axis: usize,
    dir: usize,
) {
    let mut pad = [0usize; 3];
    pad[axis] = c;
    let mut world = [0usize; 3];
    world[axis] = c - 1;

    for wi in 0..PLANE_U64S {
        let mut bits = emit[wi];
        while bits != 0 {
            let tz = bits.trailing_zeros();
            let bit_idx = wi * 64 + tz as usize;
            let t1 = bit_idx / PADDED_CHUNK_SIZE;
            let t2 = bit_idx % PADDED_CHUNK_SIZE;
            if !(1..=CHUNK_SIZE).contains(&t1) || !(1..=CHUNK_SIZE).contains(&t2) {
                bits &= bits - 1;
                continue;
            }

            let mut coords = pad;
            let mut w = world;
            match axis {
                0 => {
                    coords[1] = t1;
                    coords[2] = t2;
                    w[1] = t1 - 1;
                    w[2] = t2 - 1;
                }
                1 => {
                    coords[0] = t1;
                    coords[2] = t2;
                    w[0] = t1 - 1;
                    w[2] = t2 - 1;
                }
                _ => {
                    coords[0] = t1;
                    coords[1] = t2;
                    w[0] = t1 - 1;
                    w[1] = t2 - 1;
                }
            }

            let pi = padded_chunk_index(coords[0], coords[1], coords[2]);
            let block = blocks[pi];
            let bi = block as usize;
            let ao = compute_ao(blocks, pi, dir);
            builders[block.material_layer_index()].push_face(
                w[0],
                w[1],
                w[2],
                dir,
                tables.uvs[bi][dir],
                tables.colors[bi][dir],
                ao,
                ao_brightness,
            );

            bits &= bits - 1;
        }
    }
}

fn make_hybrid_chunk_meshes(input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
    let tables = BlockMeshTables::from_texture_map(input.block_texture_map);
    let blocks = &input.blocks.blocks;

    let data = build_bitmasks(blocks);
    let face_counts = count_faces(blocks, &data);
    let mut builders: [MeshBufferBuilder; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|i| MeshBufferBuilder::with_face_capacity(face_counts[i]));

    emit_faces(blocks, &data, &tables, input.ao_brightness, &mut builders);

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            std::mem::take(&mut builders[layer.index()])
                .into_mesh()
                .map(|mesh| (layer, mesh))
        })
        .collect()
}
