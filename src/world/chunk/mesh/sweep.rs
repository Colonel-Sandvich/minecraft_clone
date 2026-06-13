use crate::block::BlockMaterialLayer;

use super::{
    AO_SAMPLE_INDEX_OFFSETS, BlockMeshTables, BlockType, CHUNK_SIZE, ChunkLayerMeshes,
    ChunkMeshInput, ChunkMesher, MeshBufferBuilder, PADDED_CHUNK_LAYER_SIZE, PADDED_CHUNK_SIZE,
    PADDED_CHUNK_VOLUME, VERTEX_AO, padded_chunk_index, should_emit_face_from_indices,
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
    blocks[(pi as isize + offset) as usize].is_full_cube()
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
    block: BlockType,
    x: usize,
    y: usize,
    z: usize,
    dir: usize,
    ao: [u8; 4],
) {
    let bi = block as usize;
    builders[block.material_layer_index()].push_face(
        x,
        y,
        z,
        dir,
        tables.uvs[bi][dir],
        tables.colors[bi][dir],
        ao,
        ao_brightness,
    );
}

// ── Y axis: sweep along Y (layers y=0..=CHUNK_SIZE), step +1 on X axis per block ──

fn y_count(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    counts: &mut [usize; BlockMaterialLayer::COUNT],
) {
    for center in 0..=CHUNK_SIZE {
        let has_fwd = center >= 1;
        let has_back = center < CHUNK_SIZE;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(1, center, outer);
            for _inner in 0..CHUNK_SIZE {
                if has_fwd {
                    let block = blocks[pi];
                    if block.is_rendered()
                        && should_emit_face_from_indices(
                            block,
                            blocks[(pi as isize + PADDED_CHUNK_LAYER_SIZE as isize) as usize],
                        )
                    {
                        counts[block.material_layer_index()] += 1;
                    }
                }
                if has_back {
                    let b2pi = (pi as isize + PADDED_CHUNK_LAYER_SIZE as isize) as usize;
                    let block2 = blocks[b2pi];
                    if block2.is_rendered()
                        && should_emit_face_from_indices(block2, blocks[pi])
                    {
                        counts[block2.material_layer_index()] += 1;
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
        let has_back = center < CHUNK_SIZE;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(1, center, outer);
            for inner in 1..=CHUNK_SIZE {
                if has_fwd {
                    let block = blocks[pi];
                    if block.is_rendered()
                        && should_emit_face_from_indices(
                            block,
                            blocks[(pi as isize + PADDED_CHUNK_LAYER_SIZE as isize) as usize],
                        )
                    {
                        let ao = compute_ao(blocks, pi, DIR_UP);
                        emit_face(
                            builders,
                            tables,
                            ao_brightness,
                            block,
                            inner - 1,
                            center - 1,
                            outer - 1,
                            DIR_UP,
                            ao,
                        );
                    }
                }
                if has_back {
                    let b2pi = (pi as isize + PADDED_CHUNK_LAYER_SIZE as isize) as usize;
                    let block2 = blocks[b2pi];
                    if block2.is_rendered()
                        && should_emit_face_from_indices(block2, blocks[pi])
                    {
                        let ao = compute_ao(blocks, b2pi, DIR_DOWN);
                        emit_face(
                            builders,
                            tables,
                            ao_brightness,
                            block2,
                            inner - 1,
                            center,
                            outer - 1,
                            DIR_DOWN,
                            ao,
                        );
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
        let has_back = center < CHUNK_SIZE;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(center, 1, outer);
            for _inner in 0..CHUNK_SIZE {
                if has_fwd {
                    let block = blocks[pi];
                    if block.is_rendered()
                        && should_emit_face_from_indices(block, blocks[pi + 1])
                    {
                        counts[block.material_layer_index()] += 1;
                    }
                }
                if has_back {
                    let block2 = blocks[pi + 1];
                    if block2.is_rendered()
                        && should_emit_face_from_indices(block2, blocks[pi])
                    {
                        counts[block2.material_layer_index()] += 1;
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
        let has_back = center < CHUNK_SIZE;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(center, 1, outer);
            for inner in 1..=CHUNK_SIZE {
                if has_fwd {
                    let block = blocks[pi];
                    if block.is_rendered()
                        && should_emit_face_from_indices(block, blocks[pi + 1])
                    {
                        let ao = compute_ao(blocks, pi, DIR_RIGHT);
                        emit_face(
                            builders,
                            tables,
                            ao_brightness,
                            block,
                            center - 1,
                            inner - 1,
                            outer - 1,
                            DIR_RIGHT,
                            ao,
                        );
                    }
                }
                if has_back {
                    let block2 = blocks[pi + 1];
                    if block2.is_rendered()
                        && should_emit_face_from_indices(block2, blocks[pi])
                    {
                        let ao = compute_ao(blocks, pi + 1, DIR_LEFT);
                        emit_face(
                            builders,
                            tables,
                            ao_brightness,
                            block2,
                            center,
                            inner - 1,
                            outer - 1,
                            DIR_LEFT,
                            ao,
                        );
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
        let has_back = center < CHUNK_SIZE;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(1, outer, center);
            for _inner in 0..CHUNK_SIZE {
                if has_fwd {
                    let block = blocks[pi];
                    if block.is_rendered()
                        && should_emit_face_from_indices(
                            block,
                            blocks[(pi as isize + PADDED_CHUNK_SIZE as isize) as usize],
                        )
                    {
                        counts[block.material_layer_index()] += 1;
                    }
                }
                if has_back {
                    let b2pi = (pi as isize + PADDED_CHUNK_SIZE as isize) as usize;
                    let block2 = blocks[b2pi];
                    if block2.is_rendered()
                        && should_emit_face_from_indices(block2, blocks[pi])
                    {
                        counts[block2.material_layer_index()] += 1;
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
        let has_back = center < CHUNK_SIZE;
        if !has_fwd && !has_back {
            continue;
        }
        for outer in 1..=CHUNK_SIZE {
            let mut pi = padded_chunk_index(1, outer, center);
            for inner in 1..=CHUNK_SIZE {
                if has_fwd {
                    let block = blocks[pi];
                    if block.is_rendered()
                        && should_emit_face_from_indices(
                            block,
                            blocks[(pi as isize + PADDED_CHUNK_SIZE as isize) as usize],
                        )
                    {
                        let ao = compute_ao(blocks, pi, DIR_BACK);
                        emit_face(
                            builders,
                            tables,
                            ao_brightness,
                            block,
                            inner - 1,
                            outer - 1,
                            center - 1,
                            DIR_BACK,
                            ao,
                        );
                    }
                }
                if has_back {
                    let b2pi = (pi as isize + PADDED_CHUNK_SIZE as isize) as usize;
                    let block2 = blocks[b2pi];
                    if block2.is_rendered()
                        && should_emit_face_from_indices(block2, blocks[pi])
                    {
                        let ao = compute_ao(blocks, b2pi, DIR_FWD);
                        emit_face(
                            builders,
                            tables,
                            ao_brightness,
                            block2,
                            inner - 1,
                            outer - 1,
                            center,
                            DIR_FWD,
                            ao,
                        );
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
