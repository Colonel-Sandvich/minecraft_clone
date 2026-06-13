use crate::block::BlockMaterialLayer;

use super::{
    AO_SAMPLE_INDEX_OFFSETS, BLOCK_EMITS_INTERNAL_FACES, BLOCK_IS_FULL_CUBE, BLOCK_IS_RENDERED,
    BLOCK_MATERIAL_LAYER_INDEX, BlockMeshTables, BlockType, CHUNK_SIZE, ChunkLayerMeshes,
    ChunkMeshInput, ChunkMesher, DIRECTION_INDEX_OFFSETS, MeshBufferBuilder,
    PADDED_CHUNK_LAYER_SIZE, PADDED_CHUNK_SIZE, PADDED_CHUNK_VOLUME, VERTEX_AO, padded_chunk_index,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct GreedyChunkMesher;

impl ChunkMesher for GreedyChunkMesher {
    fn name(&self) -> &'static str {
        "greedy"
    }

    fn mesh(&self, input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
        if input.blocks.can_skip_mesh() {
            return Vec::new();
        }

        make_greedy_chunk_meshes(input)
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

fn plane_is_zero(plane: &BitPlane) -> bool {
    plane.iter().all(|&w| w == 0)
}

struct AxisMasks {
    full_cube: [BitPlane; PADDED_CHUNK_SIZE],
}

struct GreedyData {
    masks: [AxisMasks; 3],
    transparent_count: usize,
}

fn build_bitmasks(blocks: &[BlockType; PADDED_CHUNK_VOLUME]) -> GreedyData {
    let mut masks = std::array::from_fn(|_| AxisMasks {
        full_cube: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
    });
    let mut transparent_count = 0;

    for py in 0..PADDED_CHUNK_SIZE {
        for pz in 0..PADDED_CHUNK_SIZE {
            let mut pi = padded_chunk_index(0, py, pz);
            for px in 0..PADDED_CHUNK_SIZE {
                let i = blocks[pi] as usize;

                if BLOCK_IS_FULL_CUBE[i] {
                    let yz = plane_pack_index(py, pz);
                    plane_set(&mut masks[0].full_cube[px], yz);
                    plane_set(&mut masks[1].full_cube[py], plane_pack_index(px, pz));
                    plane_set(&mut masks[2].full_cube[pz], plane_pack_index(px, py));
                } else if BLOCK_IS_RENDERED[i] {
                    transparent_count += 1;
                }

                pi += 1;
            }
        }
    }

    GreedyData {
        masks,
        transparent_count,
    }
}

fn should_emit_transparent(block_index: usize, neighbor_index: usize) -> bool {
    if !BLOCK_IS_RENDERED[neighbor_index] {
        return true;
    }
    if BLOCK_IS_FULL_CUBE[neighbor_index] {
        return false;
    }
    if block_index == neighbor_index
        && !BLOCK_IS_FULL_CUBE[block_index]
        && !BLOCK_IS_FULL_CUBE[neighbor_index]
    {
        return BLOCK_EMITS_INTERNAL_FACES[block_index];
    }
    true
}

#[inline(always)]
fn single_vertex_ao(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    padded_index: usize,
    side_index: usize,
    vertex_index: usize,
) -> u8 {
    let offsets = AO_SAMPLE_INDEX_OFFSETS[side_index][vertex_index];
    let s1 = BLOCK_IS_FULL_CUBE[blocks[(padded_index as isize + offsets[0]) as usize] as usize];
    let s2 = BLOCK_IS_FULL_CUBE[blocks[(padded_index as isize + offsets[1]) as usize] as usize];
    let co = BLOCK_IS_FULL_CUBE[blocks[(padded_index as isize + offsets[2]) as usize] as usize];
    VERTEX_AO[s1 as usize | ((s2 as usize) << 1) | ((co as usize) << 2)]
}

const VERTEX_OFFSETS_NO_SCALE: [[(usize, usize, usize); 4]; 6] = [
    [(0, 0, 1), (0, 0, 0), (0, 1, 1), (0, 1, 0)],
    [(1, 0, 0), (1, 0, 1), (1, 1, 0), (1, 1, 1)],
    [(0, 0, 1), (1, 0, 1), (0, 0, 0), (1, 0, 0)],
    [(0, 1, 1), (0, 1, 0), (1, 1, 1), (1, 1, 0)],
    [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0)],
    [(1, 0, 1), (0, 0, 1), (1, 1, 1), (0, 1, 1)],
];

fn owning_block_coords(
    side_index: usize,
    vertex_index: usize,
    wx: usize,
    wy: usize,
    wz: usize,
    w: usize,
    h: usize,
) -> (usize, usize, usize) {
    let vo = VERTEX_OFFSETS_NO_SCALE[side_index][vertex_index];
    match side_index {
        0 | 1 => {
            let ow = if vo.2 == 0 { wz } else { wz + w - 1 };
            let oh = if vo.1 == 0 { wy } else { wy + h - 1 };
            (wx, oh, ow)
        }
        2 | 3 => {
            let ow = if vo.0 == 0 { wx } else { wx + w - 1 };
            let oh = if vo.2 == 0 { wz } else { wz + h - 1 };
            (ow, wy, oh)
        }
        _ => {
            let ow = if vo.0 == 0 { wx } else { wx + w - 1 };
            let oh = if vo.1 == 0 { wy } else { wy + h - 1 };
            (ow, oh, wz)
        }
    }
}

fn merged_ao(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    side_index: usize,
    wx: usize,
    wy: usize,
    wz: usize,
    w: usize,
    h: usize,
) -> [u8; 4] {
    std::array::from_fn(|vi| {
        let (bx, by, bz) = owning_block_coords(side_index, vi, wx, wy, wz, w, h);
        let pi = padded_chunk_index(bx + 1, by + 1, bz + 1);
        single_vertex_ao(blocks, pi, side_index, vi)
    })
}

fn count_faces(data: &GreedyData) -> [usize; BlockMaterialLayer::COUNT] {
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

    counts[1] = data.transparent_count * 6;
    counts
}

#[inline(always)]
fn face_ao_key_from_pi(blocks: &[BlockType; PADDED_CHUNK_VOLUME], pi: usize, dir: usize) -> u16 {
    let a0 = single_vertex_ao(blocks, pi, dir, 0) as u16;
    let a1 = single_vertex_ao(blocks, pi, dir, 1) as u16;
    let a2 = single_vertex_ao(blocks, pi, dir, 2) as u16;
    let a3 = single_vertex_ao(blocks, pi, dir, 3) as u16;
    a0 | (a1 << 3) | (a2 << 6) | (a3 << 9)
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
    let mut consumed = [0u64; 4];
    let packed_stride = CHUNK_SIZE;

    let (step_w, step_h, step_dw) = match axis {
        0 => (
            PADDED_CHUNK_SIZE,
            PADDED_CHUNK_LAYER_SIZE,
            PADDED_CHUNK_SIZE,
        ),
        1 => (PADDED_CHUNK_SIZE, 1, PADDED_CHUNK_SIZE),
        _ => (PADDED_CHUNK_LAYER_SIZE, 1, PADDED_CHUNK_LAYER_SIZE),
    };

    for wi in 0..PLANE_U64S {
        let mut bits = emit[wi];
        while bits != 0 {
            let tz = bits.trailing_zeros();
            let bit_idx = wi * 64 + tz as usize;
            let t1 = bit_idx / PADDED_CHUNK_SIZE;
            let t2 = bit_idx % PADDED_CHUNK_SIZE;

            bits &= bits - 1;

            if !(1..=CHUNK_SIZE).contains(&t1) || !(1..=CHUNK_SIZE).contains(&t2) {
                continue;
            }

            let packed = (t1 - 1) * packed_stride + (t2 - 1);
            let cw = packed / 64;
            let cb = packed % 64;
            if consumed[cw] & (1 << cb) != 0 {
                continue;
            }

            let (pad, world) = match axis {
                0 => ([c, t1, t2], [c - 1, t1 - 1, t2 - 1]),
                1 => ([t1, c, t2], [t1 - 1, c - 1, t2 - 1]),
                _ => ([t1, t2, c], [t1 - 1, t2 - 1, c - 1]),
            };
            let pi = padded_chunk_index(pad[0], pad[1], pad[2]);
            let bi = blocks[pi] as usize;
            let base_ao_key = face_ao_key_from_pi(blocks, pi, dir);

            let wx = world[0];
            let wy = world[1];
            let wz = world[2];

            let mut w_ext = 1;
            while t2 + w_ext <= CHUNK_SIZE {
                let p = (t1 - 1) * packed_stride + (t2 + w_ext - 1);
                if consumed[p / 64] & (1 << (p % 64)) != 0 {
                    break;
                }
                let next_bit = t1 * PADDED_CHUNK_SIZE + (t2 + w_ext);
                let nw = next_bit / 64;
                let nb = next_bit % 64;
                if (emit[nw] & (1 << nb)) == 0 {
                    break;
                }

                let np = pi + step_w * w_ext;
                if face_ao_key_from_pi(blocks, np, dir) != base_ao_key {
                    break;
                }
                if blocks[np] as usize != bi {
                    break;
                }
                w_ext += 1;
            }

            let mut h_ext = 1;
            'vloop: while t1 + h_ext <= CHUNK_SIZE {
                for dw in 0..w_ext {
                    let p = (t1 + h_ext - 1) * packed_stride + (t2 + dw - 1);
                    if consumed[p / 64] & (1 << (p % 64)) != 0 {
                        break 'vloop;
                    }
                    let next_bit = (t1 + h_ext) * PADDED_CHUNK_SIZE + (t2 + dw);
                    let nw = next_bit / 64;
                    let nb = next_bit % 64;
                    if (emit[nw] & (1 << nb)) == 0 {
                        break 'vloop;
                    }

                    let np = pi + step_h * h_ext + step_dw * dw;
                    if face_ao_key_from_pi(blocks, np, dir) != base_ao_key {
                        break 'vloop;
                    }
                    if blocks[np] as usize != bi {
                        break 'vloop;
                    }
                }
                h_ext += 1;
            }

            for dy in 0..h_ext {
                for dx in 0..w_ext {
                    let p = (t1 + dy - 1) * packed_stride + (t2 + dx - 1);
                    consumed[p / 64] |= 1 << (p % 64);
                }
            }

            let (emit_w, emit_h) = match axis {
                0 => (w_ext, h_ext),
                1 => (h_ext, w_ext),
                _ => (h_ext, w_ext),
            };

            let ao = merged_ao(blocks, dir, wx, wy, wz, emit_w, emit_h);
            builders[BLOCK_MATERIAL_LAYER_INDEX[bi]].push_merged_face(
                wx,
                wy,
                wz,
                emit_w,
                emit_h,
                dir,
                tables.uvs[bi][dir],
                tables.colors[bi][dir],
                ao,
                ao_brightness,
            );
        }
    }
}

fn emit_faces(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    data: &GreedyData,
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
            if !plane_is_zero(&emit_first) {
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
            }

            let emit_second =
                bitwise_and_not(&axis_masks.full_cube[c], &axis_masks.full_cube[c + 1]);
            if !plane_is_zero(&emit_second) {
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
    }

    if data.transparent_count > 0 {
        for px in 1..=CHUNK_SIZE {
            for py in 1..=CHUNK_SIZE {
                for pz in 1..=CHUNK_SIZE {
                    let pi = padded_chunk_index(px, py, pz);
                    let bi = blocks[pi] as usize;

                    if BLOCK_IS_FULL_CUBE[bi] || !BLOCK_IS_RENDERED[bi] {
                        continue;
                    }

                    let wx = px - 1;
                    let wy = py - 1;
                    let wz = pz - 1;

                    for dir in 0..6usize {
                        let ni =
                            blocks[(pi as isize + DIRECTION_INDEX_OFFSETS[dir]) as usize] as usize;
                        if should_emit_transparent(bi, ni) {
                            let ao = [
                                single_vertex_ao(blocks, pi, dir, 0),
                                single_vertex_ao(blocks, pi, dir, 1),
                                single_vertex_ao(blocks, pi, dir, 2),
                                single_vertex_ao(blocks, pi, dir, 3),
                            ];
                            builders[BLOCK_MATERIAL_LAYER_INDEX[bi]].push_face(
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

fn make_greedy_chunk_meshes(input: ChunkMeshInput<'_>) -> ChunkLayerMeshes {
    let tables = BlockMeshTables::from_texture_map(input.block_texture_map);
    let blocks = &input.blocks.blocks;

    let data = build_bitmasks(blocks);
    let face_counts = count_faces(&data);
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
