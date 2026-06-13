use crate::block::BlockMaterialLayer;

use super::{
    BLOCK_IS_FULL_CUBE, BLOCK_IS_RENDERED, BLOCK_MATERIAL_LAYER_INDEX, AO_SAMPLE_INDEX_OFFSETS,
    BlockMeshTables, BlockType, CHUNK_SIZE, ChunkLayerMeshes, ChunkMeshInput, ChunkMesher,
    MeshBufferBuilder, PADDED_CHUNK_LAYER_SIZE, PADDED_CHUNK_SIZE, PADDED_CHUNK_VOLUME, VERTEX_AO,
    padded_chunk_index, should_emit_face_from_indices,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct SweepChunkMesher;

impl ChunkMesher for SweepChunkMesher {
    fn name(&self) -> &'static str {
        "sweep"
    }

    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
        if input.blocks.can_skip_mesh() {
            return Vec::new();
        }
        make_sweep_chunk_meshes(input)
    }
}

const DIR_UP: usize = 3;
const DIR_DOWN: usize = 2;
const DIR_RIGHT: usize = 1;
const DIR_LEFT: usize = 0;
const DIR_BACK: usize = 5;
const DIR_FWD: usize = 4;

fn block_occludes(blocks: &[BlockType; PADDED_CHUNK_VOLUME], pi: usize, offset: isize) -> bool {
    BLOCK_IS_FULL_CUBE[blocks[(pi as isize + offset) as usize] as usize]
}

fn compute_ao(blocks: &[BlockType; PADDED_CHUNK_VOLUME], pi: usize, dir: usize) -> [u8; 4] {
    AO_SAMPLE_INDEX_OFFSETS[dir].map(|o| {
        let s1 = block_occludes(blocks, pi, o[0]);
        let s2 = block_occludes(blocks, pi, o[1]);
        let co = block_occludes(blocks, pi, o[2]);
        VERTEX_AO[s1 as usize | ((s2 as usize) << 1) | ((co as usize) << 2)]
    })
}

#[inline(always)]
fn emit_face(
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    bi: usize,
    x: usize,
    y: usize,
    z: usize,
    dir: usize,
    ao: [u8; 4],
) {
    builders[BLOCK_MATERIAL_LAYER_INDEX[bi]].push_face(
        x, y, z, dir,
        tables.uvs[bi][dir], tables.colors[bi][dir], ao, ao_brightness,
    );
}

// ── Y axis: sweep along Y (layers y=0..=CHUNK_SIZE), step +1 on X axis per block ──

fn y_count(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    counts: &mut [usize; BlockMaterialLayer::COUNT],
) {
    for center in 0..=CHUNK_SIZE {
        let has_fwd = center >= 1;
        let has_back = center <= CHUNK_SIZE - 1;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(1, center, outer);
            for _inner in 0..CHUNK_SIZE {
                if has_fwd {
                    let bi = blocks[pi] as usize;
                    if BLOCK_IS_RENDERED[bi]
                        && should_emit_face_from_indices(bi, blocks[(pi as isize + PADDED_CHUNK_LAYER_SIZE as isize) as usize] as usize, DIR_UP)
                    {
                        counts[BLOCK_MATERIAL_LAYER_INDEX[bi]] += 1;
                    }
                }
                if has_back {
                    let b2pi = (pi as isize + PADDED_CHUNK_LAYER_SIZE as isize) as usize;
                    let b2i = blocks[b2pi] as usize;
                    if BLOCK_IS_RENDERED[b2i]
                        && should_emit_face_from_indices(b2i, blocks[pi] as usize, DIR_DOWN)
                    {
                        counts[BLOCK_MATERIAL_LAYER_INDEX[b2i]] += 1;
                    }
                }
                pi += 1;
            }
        }
    }
}

fn y_emit(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
) {
    for center in 0..=CHUNK_SIZE {
        let has_fwd = center >= 1;
        let has_back = center <= CHUNK_SIZE - 1;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(1, center, outer);
            for inner in 1..=CHUNK_SIZE {
                if has_fwd {
                    let bi = blocks[pi] as usize;
                    if BLOCK_IS_RENDERED[bi]
                        && should_emit_face_from_indices(bi, blocks[(pi as isize + PADDED_CHUNK_LAYER_SIZE as isize) as usize] as usize, DIR_UP)
                    {
                        let ao = compute_ao(blocks, pi, DIR_UP);
                        emit_face(builders, tables, ao_brightness, bi,
                            inner - 1, center - 1, outer - 1, DIR_UP, ao);
                    }
                }
                if has_back {
                    let b2pi = (pi as isize + PADDED_CHUNK_LAYER_SIZE as isize) as usize;
                    let b2i = blocks[b2pi] as usize;
                    if BLOCK_IS_RENDERED[b2i]
                        && should_emit_face_from_indices(b2i, blocks[pi] as usize, DIR_DOWN)
                    {
                        let ao = compute_ao(blocks, b2pi, DIR_DOWN);
                        emit_face(builders, tables, ao_brightness, b2i,
                            inner - 1, center, outer - 1, DIR_DOWN, ao);
                    }
                }
                pi += 1;
            }
        }
    }
}

// ── X axis: sweep along X (layers x=0..=CHUNK_SIZE), step +PADDED_CHUNK_LAYER_SIZE on Y ──

fn x_count(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    counts: &mut [usize; BlockMaterialLayer::COUNT],
) {
    for center in 0..=CHUNK_SIZE {
        let has_fwd = center >= 1;
        let has_back = center <= CHUNK_SIZE - 1;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(center, 1, outer);
            for _inner in 0..CHUNK_SIZE {
                if has_fwd {
                    let bi = blocks[pi] as usize;
                    if BLOCK_IS_RENDERED[bi]
                        && should_emit_face_from_indices(bi, blocks[(pi + 1) as usize] as usize, DIR_RIGHT)
                    {
                        counts[BLOCK_MATERIAL_LAYER_INDEX[bi]] += 1;
                    }
                }
                if has_back {
                    let b2i = blocks[pi + 1] as usize;
                    if BLOCK_IS_RENDERED[b2i]
                        && should_emit_face_from_indices(b2i, blocks[pi] as usize, DIR_LEFT)
                    {
                        counts[BLOCK_MATERIAL_LAYER_INDEX[b2i]] += 1;
                    }
                }
                pi += PADDED_CHUNK_LAYER_SIZE;
            }
        }
    }
}

fn x_emit(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
) {
    for center in 0..=CHUNK_SIZE {
        let has_fwd = center >= 1;
        let has_back = center <= CHUNK_SIZE - 1;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(center, 1, outer);
            for inner in 1..=CHUNK_SIZE {
                if has_fwd {
                    let bi = blocks[pi] as usize;
                    if BLOCK_IS_RENDERED[bi]
                        && should_emit_face_from_indices(bi, blocks[(pi + 1) as usize] as usize, DIR_RIGHT)
                    {
                        let ao = compute_ao(blocks, pi, DIR_RIGHT);
                        emit_face(builders, tables, ao_brightness, bi,
                            center - 1, inner - 1, outer - 1, DIR_RIGHT, ao);
                    }
                }
                if has_back {
                    let b2i = blocks[pi + 1] as usize;
                    if BLOCK_IS_RENDERED[b2i]
                        && should_emit_face_from_indices(b2i, blocks[pi] as usize, DIR_LEFT)
                    {
                        let ao = compute_ao(blocks, pi + 1, DIR_LEFT);
                        emit_face(builders, tables, ao_brightness, b2i,
                            center, inner - 1, outer - 1, DIR_LEFT, ao);
                    }
                }
                pi += PADDED_CHUNK_LAYER_SIZE;
            }
        }
    }
}

// ── Z axis: sweep along Z (layers z=0..=CHUNK_SIZE), step +1 on X axis per block ──

fn z_count(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    counts: &mut [usize; BlockMaterialLayer::COUNT],
) {
    for center in 0..=CHUNK_SIZE {
        let has_fwd = center >= 1;
        let has_back = center <= CHUNK_SIZE - 1;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(1, outer, center);
            for _inner in 0..CHUNK_SIZE {
                if has_fwd {
                    let bi = blocks[pi] as usize;
                    if BLOCK_IS_RENDERED[bi]
                        && should_emit_face_from_indices(bi, blocks[(pi as isize + PADDED_CHUNK_SIZE as isize) as usize] as usize, DIR_BACK)
                    {
                        counts[BLOCK_MATERIAL_LAYER_INDEX[bi]] += 1;
                    }
                }
                if has_back {
                    let b2pi = (pi as isize + PADDED_CHUNK_SIZE as isize) as usize;
                    let b2i = blocks[b2pi] as usize;
                    if BLOCK_IS_RENDERED[b2i]
                        && should_emit_face_from_indices(b2i, blocks[pi] as usize, DIR_FWD)
                    {
                        counts[BLOCK_MATERIAL_LAYER_INDEX[b2i]] += 1;
                    }
                }
                pi += 1;
            }
        }
    }
}

fn z_emit(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
) {
    for center in 0..=CHUNK_SIZE {
        let has_fwd = center >= 1;
        let has_back = center <= CHUNK_SIZE - 1;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(1, outer, center);
            for inner in 1..=CHUNK_SIZE {
                if has_fwd {
                    let bi = blocks[pi] as usize;
                    if BLOCK_IS_RENDERED[bi]
                        && should_emit_face_from_indices(bi, blocks[(pi as isize + PADDED_CHUNK_SIZE as isize) as usize] as usize, DIR_BACK)
                    {
                        let ao = compute_ao(blocks, pi, DIR_BACK);
                        emit_face(builders, tables, ao_brightness, bi,
                            inner - 1, outer - 1, center - 1, DIR_BACK, ao);
                    }
                }
                if has_back {
                    let b2pi = (pi as isize + PADDED_CHUNK_SIZE as isize) as usize;
                    let b2i = blocks[b2pi] as usize;
                    if BLOCK_IS_RENDERED[b2i]
                        && should_emit_face_from_indices(b2i, blocks[pi] as usize, DIR_FWD)
                    {
                        let ao = compute_ao(blocks, b2pi, DIR_FWD);
                        emit_face(builders, tables, ao_brightness, b2i,
                            inner - 1, outer - 1, center, DIR_FWD, ao);
                    }
                }
                pi += 1;
            }
        }
    }
}

// ── Main entry point ──

fn make_sweep_chunk_meshes(input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
    let tables = BlockMeshTables::from_texture_map(input.block_texture_map);
    let blocks = &input.blocks.blocks;

    let mut counts = [0; BlockMaterialLayer::COUNT];
    y_count(blocks, &mut counts);
    x_count(blocks, &mut counts);
    z_count(blocks, &mut counts);

    let mut builders: [MeshBufferBuilder; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|i| MeshBufferBuilder::with_face_capacity(counts[i]));

    y_emit(blocks, &tables, input.ao_brightness, &mut builders);
    x_emit(blocks, &tables, input.ao_brightness, &mut builders);
    z_emit(blocks, &tables, input.ao_brightness, &mut builders);

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            std::mem::take(&mut builders[layer.index()])
                .into_mesh()
                .map(|mesh| (layer, mesh))
        })
        .collect()
}
