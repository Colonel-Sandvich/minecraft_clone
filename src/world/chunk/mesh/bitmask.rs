use crate::block::BlockMaterialLayer;

use super::{
    BLOCK_IS_FULL_CUBE, BLOCK_IS_RENDERED, BLOCK_MATERIAL_LAYER_INDEX, AO_SAMPLE_INDEX_OFFSETS,
    BlockMeshTables, BlockType, CHUNK_SIZE, ChunkLayerMeshes, ChunkMeshInput, ChunkMesher,
    DIRECTION_INDEX_OFFSETS, MeshBufferBuilder, PADDED_CHUNK_SIZE,
    PADDED_CHUNK_VOLUME, VERTEX_AO, padded_chunk_index, should_emit_face_from_indices,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct BitmaskChunkMesher;

impl ChunkMesher for BitmaskChunkMesher {
    fn name(&self) -> &'static str {
        "bitmask"
    }

    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
        if input.blocks.can_skip_mesh() {
            return Vec::new();
        }

        make_bitmask_chunk_meshes(input)
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

struct PerAxisMasks {
    rendered: [BitPlane; PADDED_CHUNK_SIZE],
    full_cube: [BitPlane; PADDED_CHUNK_SIZE],
}

struct BitmaskData {
    x: PerAxisMasks,
    y: PerAxisMasks,
    z: PerAxisMasks,
}

fn build_bitmasks(blocks: &[BlockType; PADDED_CHUNK_VOLUME]) -> BitmaskData {
    let mut d = BitmaskData {
        x: PerAxisMasks {
            rendered: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
            full_cube: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
        },
        y: PerAxisMasks {
            rendered: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
            full_cube: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
        },
        z: PerAxisMasks {
            rendered: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
            full_cube: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
        },
    };

    for py in 0..PADDED_CHUNK_SIZE {
        for pz in 0..PADDED_CHUNK_SIZE {
            let mut pi = padded_chunk_index(0, py, pz);
            for px in 0..PADDED_CHUNK_SIZE {
                let b = blocks[pi];
                let i = b as usize;

                let yz = plane_pack_index(py, pz);
                if BLOCK_IS_RENDERED[i] {
                    plane_set(&mut d.x.rendered[px], yz);
                }
                if BLOCK_IS_FULL_CUBE[i] {
                    plane_set(&mut d.x.full_cube[px], yz);
                }

                let xz = plane_pack_index(px, pz);
                if BLOCK_IS_RENDERED[i] {
                    plane_set(&mut d.y.rendered[py], xz);
                }
                if BLOCK_IS_FULL_CUBE[i] {
                    plane_set(&mut d.y.full_cube[py], xz);
                }

                let xy = plane_pack_index(px, py);
                if BLOCK_IS_RENDERED[i] {
                    plane_set(&mut d.z.rendered[pz], xy);
                }
                if BLOCK_IS_FULL_CUBE[i] {
                    plane_set(&mut d.z.full_cube[pz], xy);
                }

                pi += 1;
            }
        }
    }

    d
}

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

fn count_axis(
    m: &PerAxisMasks,
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    counts: &mut [usize; BlockMaterialLayer::COUNT],
    ca: usize,
    t1a: usize,
    t2a: usize,
    first_dir: usize,
    second_dir: usize,
) {
    for c in 1..=CHUNK_SIZE {
        let e1 = bitwise_and_not(&m.rendered[c], &m.full_cube[c - 1]);
        count_plane(&e1, blocks, counts, c, ca, t1a, t2a, first_dir);
        let e2 = bitwise_and_not(&m.rendered[c], &m.full_cube[c + 1]);
        count_plane(&e2, blocks, counts, c, ca, t1a, t2a, second_dir);
    }
}

fn count_plane(
    emit: &BitPlane,
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    counts: &mut [usize; BlockMaterialLayer::COUNT],
    c: usize,
    ca: usize,
    t1a: usize,
    t2a: usize,
    dir: usize,
) {
    let mut pad = [0_usize; 3];
    pad[ca] = c;

    for wi in 0..PLANE_U64S {
        let mut bits = emit[wi];
        while bits != 0 {
            let tz = bits.trailing_zeros();
            let bit_idx = wi * 64 + tz as usize;
            let t1 = bit_idx / PADDED_CHUNK_SIZE;
            let t2 = bit_idx % PADDED_CHUNK_SIZE;
            pad[t1a] = t1;
            pad[t2a] = t2;

            let pi = padded_chunk_index(pad[0], pad[1], pad[2]);
            let bi = blocks[pi] as usize;
            let ni = blocks[(pi as isize + DIRECTION_INDEX_OFFSETS[dir]) as usize] as usize;

            if should_emit_face_from_indices(bi, ni, dir) {
                counts[BLOCK_MATERIAL_LAYER_INDEX[bi]] += 1;
            }

            bits &= bits - 1;
        }
    }
}

fn count_bitmask_faces(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    m: &BitmaskData,
) -> [usize; BlockMaterialLayer::COUNT] {
    let mut counts = [0; BlockMaterialLayer::COUNT];
    count_axis(&m.x, blocks, &mut counts, 0, 1, 2, 0, 1);
    count_axis(&m.y, blocks, &mut counts, 1, 0, 2, 2, 3);
    count_axis(&m.z, blocks, &mut counts, 2, 0, 1, 4, 5);
    counts
}

fn emit_axis(
    m: &PerAxisMasks,
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
    ca: usize,
    t1a: usize,
    t2a: usize,
    first_dir: usize,
    second_dir: usize,
) {
    for c in 1..=CHUNK_SIZE {
        let e1 = bitwise_and_not(&m.rendered[c], &m.full_cube[c - 1]);
        emit_plane(
            &e1, blocks, tables, ao_brightness, builders, c, ca, t1a, t2a, first_dir,
        );
        let e2 = bitwise_and_not(&m.rendered[c], &m.full_cube[c + 1]);
        emit_plane(
            &e2, blocks, tables, ao_brightness, builders, c, ca, t1a, t2a, second_dir,
        );
    }
}

fn emit_plane(
    emit: &BitPlane,
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
    c: usize,
    ca: usize,
    t1a: usize,
    t2a: usize,
    dir: usize,
) {
    let mut pad = [0_usize; 3];
    pad[ca] = c;
    let mut world = [0_usize; 3];
    world[ca] = c - 1;

    for wi in 0..PLANE_U64S {
        let mut bits = emit[wi];
        while bits != 0 {
            let tz = bits.trailing_zeros();
            let bit_idx = wi * 64 + tz as usize;
            let t1 = bit_idx / PADDED_CHUNK_SIZE;
            let t2 = bit_idx % PADDED_CHUNK_SIZE;
            pad[t1a] = t1;
            pad[t2a] = t2;
            world[t1a] = t1 - 1;
            world[t2a] = t2 - 1;

            let pi = padded_chunk_index(pad[0], pad[1], pad[2]);
            let bi = blocks[pi] as usize;
            let ni = blocks[(pi as isize + DIRECTION_INDEX_OFFSETS[dir]) as usize] as usize;

            if should_emit_face_from_indices(bi, ni, dir) {
                let ao = compute_ao(blocks, pi, dir);
                builders[BLOCK_MATERIAL_LAYER_INDEX[bi]].push_face(
                    world[0], world[1], world[2], dir,
                    tables.uvs[bi][dir], tables.colors[bi][dir], ao, ao_brightness,
                );
            }

            bits &= bits - 1;
        }
    }
}

fn make_bitmask_chunk_meshes(input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
    let tables = BlockMeshTables::from_texture_map(input.block_texture_map);
    let blocks = &input.blocks.blocks;

    let masks = build_bitmasks(blocks);
    let face_counts = count_bitmask_faces(blocks, &masks);
    let mut builders: [MeshBufferBuilder; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|i| MeshBufferBuilder::with_face_capacity(face_counts[i]));

    emit_axis(&masks.x, blocks, &tables, input.ao_brightness, &mut builders, 0, 1, 2, 0, 1);
    emit_axis(&masks.y, blocks, &tables, input.ao_brightness, &mut builders, 1, 0, 2, 2, 3);
    emit_axis(&masks.z, blocks, &tables, input.ao_brightness, &mut builders, 2, 0, 1, 4, 5);

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            std::mem::take(&mut builders[layer.index()])
                .into_mesh()
                .map(|mesh| (layer, mesh))
        })
        .collect()
}
