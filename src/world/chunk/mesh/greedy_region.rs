//! Region-based greedy meshing: meshes 2×2×2 chunks (32³ blocks) in one pass.
//!
//! The algorithm is identical to single-chunk greedy (`greedy.rs`) but with
//! scaled dimensions (34³ padded, 19 u64s/bitplane).  This produces larger
//! quads (up to 32×32) and eliminates interior faces between chunks within
//! the region.
//!
//! AO offsets are generated from the padded dimensions at mesh time because
//! they depend on `PADDED_LAYER_SIZE`; the 72-entry table costs <1 µs to build.

use crate::block::BlockMaterialLayer;

use super::{
    BlockMeshTables, BlockTextureMap, BlockType, ChunkLayerMeshes, MeshBufferBuilder, VERTEX_AO,
    should_emit_face_from_indices,
};

use crate::world::chunk::{CHUNK_ISIZE, CHUNK_SIZE, Chunk};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

pub const REGION_CHUNKS: usize = 2;
pub const REGION_SIZE: usize = REGION_CHUNKS * 16;
pub const PADDED_REGION_SIZE: usize = REGION_SIZE + 2;
pub const PADDED_REGION_VOLUME: usize =
    PADDED_REGION_SIZE * PADDED_REGION_SIZE * PADDED_REGION_SIZE;
pub const PADDED_REGION_LAYER_SIZE: usize = PADDED_REGION_SIZE * PADDED_REGION_SIZE;

const PLANE_U64S: usize = PADDED_REGION_LAYER_SIZE.div_ceil(64);
type BitPlane = [u64; PLANE_U64S];

const FULL_MASK_U64S: usize = PADDED_REGION_VOLUME.div_ceil(64);

#[derive(Clone, Copy)]
struct FullCubeMask([u64; FULL_MASK_U64S]);

impl FullCubeMask {
    #[inline(always)]
    #[allow(dead_code)]
    fn is_full_cube(&self, idx: usize) -> bool {
        (self.0[idx / 64] >> (idx % 64)) & 1 != 0
    }
}

#[inline(always)]
fn padded_region_index(x: usize, y: usize, z: usize) -> usize {
    x + PADDED_REGION_SIZE * (z + PADDED_REGION_SIZE * y)
}

fn plane_pack_index(a: usize, b: usize) -> usize {
    a * PADDED_REGION_SIZE + b
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

fn make_direction_offsets() -> [isize; 6] {
    let ps = PADDED_REGION_SIZE as isize;
    let plr = PADDED_REGION_LAYER_SIZE as isize;
    [-1, 1, -plr, plr, -ps, ps]
}

fn make_ao_offsets() -> [[[isize; 3]; 4]; 6] {
    let ps = PADDED_REGION_SIZE as isize;
    let plr = PADDED_REGION_LAYER_SIZE as isize;

    let flat = |dx: isize, dy: isize, dz: isize| -> isize { dx + ps * dz + plr * dy };

    let raw: [[[(isize, isize, isize); 3]; 4]; 6] = [
        [
            [(-1, -1, 0), (-1, 0, 1), (-1, -1, 1)],
            [(-1, -1, 0), (-1, 0, -1), (-1, -1, -1)],
            [(-1, 1, 0), (-1, 0, 1), (-1, 1, 1)],
            [(-1, 1, 0), (-1, 0, -1), (-1, 1, -1)],
        ],
        [
            [(1, -1, 0), (1, 0, -1), (1, -1, -1)],
            [(1, -1, 0), (1, 0, 1), (1, -1, 1)],
            [(1, 1, 0), (1, 0, -1), (1, 1, -1)],
            [(1, 1, 0), (1, 0, 1), (1, 1, 1)],
        ],
        [
            [(-1, -1, 0), (0, -1, 1), (-1, -1, 1)],
            [(1, -1, 0), (0, -1, 1), (1, -1, 1)],
            [(-1, -1, 0), (0, -1, -1), (-1, -1, -1)],
            [(1, -1, 0), (0, -1, -1), (1, -1, -1)],
        ],
        [
            [(-1, 1, 0), (0, 1, 1), (-1, 1, 1)],
            [(-1, 1, 0), (0, 1, -1), (-1, 1, -1)],
            [(1, 1, 0), (0, 1, 1), (1, 1, 1)],
            [(1, 1, 0), (0, 1, -1), (1, 1, -1)],
        ],
        [
            [(-1, 0, -1), (0, -1, -1), (-1, -1, -1)],
            [(1, 0, -1), (0, -1, -1), (1, -1, -1)],
            [(-1, 0, -1), (0, 1, -1), (-1, 1, -1)],
            [(1, 0, -1), (0, 1, -1), (1, 1, -1)],
        ],
        [
            [(1, 0, 1), (0, -1, 1), (1, -1, 1)],
            [(-1, 0, 1), (0, -1, 1), (-1, -1, 1)],
            [(1, 0, 1), (0, 1, 1), (1, 1, 1)],
            [(-1, 0, 1), (0, 1, 1), (-1, 1, 1)],
        ],
    ];

    std::array::from_fn(|dir| {
        std::array::from_fn(|vi| {
            std::array::from_fn(|si| {
                let (dx, dy, dz) = raw[dir][vi][si];
                flat(dx, dy, dz)
            })
        })
    })
}

struct AxisMasks {
    full_cube: [BitPlane; PADDED_REGION_SIZE],
    rendered_non_full: [BitPlane; PADDED_REGION_SIZE],
}

struct GreedyData {
    masks: [AxisMasks; 3],
    transparent_count: usize,
    full_mask: FullCubeMask,
    ao_offsets: [[[isize; 3]; 4]; 6],
    dir_offsets: [isize; 6],
}

fn build_bitmasks(
    blocks: &[BlockType; PADDED_REGION_VOLUME],
    ao_offsets: [[[isize; 3]; 4]; 6],
    dir_offsets: [isize; 6],
) -> GreedyData {
    let mut masks = std::array::from_fn(|_| AxisMasks {
        full_cube: [[0u64; PLANE_U64S]; PADDED_REGION_SIZE],
        rendered_non_full: [[0u64; PLANE_U64S]; PADDED_REGION_SIZE],
    });
    let mut full_mask = [0u64; FULL_MASK_U64S];
    let mut transparent_count = 0;

    for py in 0..PADDED_REGION_SIZE {
        for pz in 0..PADDED_REGION_SIZE {
            let mut pi = padded_region_index(0, py, pz);
            for px in 0..PADDED_REGION_SIZE {
                let block = blocks[pi];

                if block.is_full_cube() {
                    full_mask[pi / 64] |= 1 << (pi % 64);
                    let yz = plane_pack_index(py, pz);
                    plane_set(&mut masks[0].full_cube[px], yz);
                    plane_set(&mut masks[1].full_cube[py], plane_pack_index(px, pz));
                    plane_set(&mut masks[2].full_cube[pz], plane_pack_index(px, py));
                } else if block.is_rendered() {
                    transparent_count += 1;
                    let yz = plane_pack_index(py, pz);
                    plane_set(&mut masks[0].rendered_non_full[px], yz);
                    plane_set(
                        &mut masks[1].rendered_non_full[py],
                        plane_pack_index(px, pz),
                    );
                    plane_set(
                        &mut masks[2].rendered_non_full[pz],
                        plane_pack_index(px, py),
                    );
                }

                pi += 1;
            }
        }
    }

    GreedyData {
        masks,
        transparent_count,
        full_mask: FullCubeMask(full_mask),
        ao_offsets,
        dir_offsets,
    }
}

fn count_faces(data: &GreedyData) -> [usize; BlockMaterialLayer::COUNT] {
    let mut counts = [0; BlockMaterialLayer::COUNT];

    for axis in 0..3usize {
        let axis_masks = &data.masks[axis];

        for c in 1..=REGION_SIZE {
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
fn single_vertex_ao(
    full_mask: &FullCubeMask,
    padded_index: usize,
    side_index: usize,
    vertex_index: usize,
    ao_offsets: &[[[isize; 3]; 4]; 6],
) -> u8 {
    let offsets = ao_offsets[side_index][vertex_index];
    let m = &full_mask.0;
    let idx0 = (padded_index as isize + offsets[0]) as usize;
    let idx1 = (padded_index as isize + offsets[1]) as usize;
    let idx2 = (padded_index as isize + offsets[2]) as usize;
    let s1 = (m[idx0 >> 6] >> (idx0 & 63)) & 1;
    let s2 = (m[idx1 >> 6] >> (idx1 & 63)) & 1;
    let co = (m[idx2 >> 6] >> (idx2 & 63)) & 1;
    VERTEX_AO[s1 as usize | ((s2 as usize) << 1) | ((co as usize) << 2)]
}

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
    full_mask: &FullCubeMask,
    side_index: usize,
    wx: usize,
    wy: usize,
    wz: usize,
    w: usize,
    h: usize,
    ao_offsets: &[[[isize; 3]; 4]; 6],
) -> [u8; 4] {
    std::array::from_fn(|vi| {
        let (bx, by, bz) = owning_block_coords(side_index, vi, wx, wy, wz, w, h);
        let pi = padded_region_index(bx + 1, by + 1, bz + 1);
        single_vertex_ao(full_mask, pi, side_index, vi, ao_offsets)
    })
}

#[inline(always)]
fn face_ao_key_from_pi(
    full_mask: &FullCubeMask,
    pi: usize,
    dir: usize,
    ao_offsets: &[[[isize; 3]; 4]; 6],
) -> u8 {
    let a0 = single_vertex_ao(full_mask, pi, dir, 0, ao_offsets);
    let a1 = single_vertex_ao(full_mask, pi, dir, 1, ao_offsets);
    let a2 = single_vertex_ao(full_mask, pi, dir, 2, ao_offsets);
    let a3 = single_vertex_ao(full_mask, pi, dir, 3, ao_offsets);
    a0 | (a1 << 2) | (a2 << 4) | (a3 << 6)
}

#[inline(always)]
fn unpack_ao_key(key: u8) -> [u8; 4] {
    [key & 3, (key >> 2) & 3, (key >> 4) & 3, key >> 6]
}

const VERTEX_OFFSETS_NO_SCALE: [[(usize, usize, usize); 4]; 6] = [
    [(0, 0, 1), (0, 0, 0), (0, 1, 1), (0, 1, 0)],
    [(1, 0, 0), (1, 0, 1), (1, 1, 0), (1, 1, 1)],
    [(0, 0, 1), (1, 0, 1), (0, 0, 0), (1, 0, 0)],
    [(0, 1, 1), (0, 1, 0), (1, 1, 1), (1, 1, 0)],
    [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0)],
    [(1, 0, 1), (0, 0, 1), (1, 1, 1), (0, 1, 1)],
];

fn emit_plane_opaque<const IS_TRANSPARENT: bool>(
    emit: &BitPlane,
    blocks: &[BlockType; PADDED_REGION_VOLUME],
    full_mask: &FullCubeMask,
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
    c: usize,
    axis: usize,
    dir: usize,
    ao_offsets: &[[[isize; 3]; 4]; 6],
    dir_offsets: &[isize; 6],
) {
    let mut consumed = [0u64; 16];
    let packed_stride = REGION_SIZE;

    let (step_w, step_h, step_dw) = match axis {
        0 => (
            PADDED_REGION_SIZE,
            PADDED_REGION_LAYER_SIZE,
            PADDED_REGION_SIZE,
        ),
        1 => (PADDED_REGION_SIZE, 1, PADDED_REGION_SIZE),
        _ => (PADDED_REGION_LAYER_SIZE, 1, PADDED_REGION_LAYER_SIZE),
    };

    for wi in 0..PLANE_U64S {
        let mut bits = emit[wi];
        while bits != 0 {
            let tz = bits.trailing_zeros();
            let bit_idx = (wi * 64) + tz as usize;
            let t1 = bit_idx / PADDED_REGION_SIZE;
            let t2 = bit_idx % PADDED_REGION_SIZE;

            bits &= bits - 1;

            if !(1..=REGION_SIZE).contains(&t1) || !(1..=REGION_SIZE).contains(&t2) {
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
            let pi = padded_region_index(pad[0], pad[1], pad[2]);
            let block = blocks[pi];

            if IS_TRANSPARENT {
                let exterior_pi = (pi as isize + dir_offsets[dir]) as usize;
                if !should_emit_face_from_indices(block, blocks[exterior_pi]) {
                    consumed[cw] |= 1 << cb;
                    continue;
                }
            }

            let base_ao_key = face_ao_key_from_pi(full_mask, pi, dir, ao_offsets);

            let wx = world[0];
            let wy = world[1];
            let wz = world[2];

            let row_start_bit = t1 * PADDED_REGION_SIZE + t2;
            let rw = row_start_bit / 64;
            let rb = row_start_bit % 64;
            let word_bits = emit[rw] >> rb;
            let max_emit_run = ((word_bits.trailing_ones() as usize) - 1)
                .min(REGION_SIZE - t2)
                .min(64 - rb - 1);

            let mut w_ext = 1;
            while w_ext <= max_emit_run {
                let p = (t1 - 1) * packed_stride + (t2 + w_ext - 1);
                if consumed[p / 64] & (1 << (p % 64)) != 0 {
                    break;
                }

                let np = pi + step_w * w_ext;
                if IS_TRANSPARENT {
                    let exterior_np = (np as isize + dir_offsets[dir]) as usize;
                    if !should_emit_face_from_indices(blocks[np], blocks[exterior_np]) {
                        break;
                    }
                }
                if face_ao_key_from_pi(full_mask, np, dir, ao_offsets) != base_ao_key {
                    break;
                }
                if blocks[np] != block {
                    break;
                }
                w_ext += 1;
            }

            let mut h_ext = 1;
            'vloop: while t1 + h_ext <= REGION_SIZE {
                for dw in 0..w_ext {
                    let p = (t1 + h_ext - 1) * packed_stride + (t2 + dw - 1);
                    if consumed[p / 64] & (1 << (p % 64)) != 0 {
                        break 'vloop;
                    }
                    let next_bit = (t1 + h_ext) * PADDED_REGION_SIZE + (t2 + dw);
                    let nw = next_bit / 64;
                    let nb = next_bit % 64;
                    if (emit[nw] & (1 << nb)) == 0 {
                        break 'vloop;
                    }

                    let np = pi + step_h * h_ext + step_dw * dw;
                    if IS_TRANSPARENT {
                        let exterior_np = (np as isize + dir_offsets[dir]) as usize;
                        if !should_emit_face_from_indices(blocks[np], blocks[exterior_np]) {
                            break 'vloop;
                        }
                    }
                    if face_ao_key_from_pi(full_mask, np, dir, ao_offsets) != base_ao_key {
                        break 'vloop;
                    }
                    if blocks[np] != block {
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

            let bi = block as usize;
            let ao = if w_ext == 1 && h_ext == 1 {
                unpack_ao_key(base_ao_key)
            } else {
                merged_ao(full_mask, dir, wx, wy, wz, emit_w, emit_h, ao_offsets)
            };
            builders[block.material_layer_index()].push_merged_face(
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
    blocks: &[BlockType; PADDED_REGION_VOLUME],
    data: &GreedyData,
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
) {
    let full_mask = &data.full_mask;
    let ao_offsets = &data.ao_offsets;
    let dir_offsets = &data.dir_offsets;

    for axis in 0..3usize {
        let axis_masks = &data.masks[axis];
        let first_dir = axis * 2;
        let second_dir = axis * 2 + 1;

        for c in 1..=REGION_SIZE {
            let emit_first =
                bitwise_and_not(&axis_masks.full_cube[c], &axis_masks.full_cube[c - 1]);
            if !plane_is_zero(&emit_first) {
                emit_plane_opaque::<false>(
                    &emit_first,
                    blocks,
                    full_mask,
                    tables,
                    ao_brightness,
                    builders,
                    c,
                    axis,
                    first_dir,
                    ao_offsets,
                    dir_offsets,
                );
            }

            let emit_second =
                bitwise_and_not(&axis_masks.full_cube[c], &axis_masks.full_cube[c + 1]);
            if !plane_is_zero(&emit_second) {
                emit_plane_opaque::<false>(
                    &emit_second,
                    blocks,
                    full_mask,
                    tables,
                    ao_brightness,
                    builders,
                    c,
                    axis,
                    second_dir,
                    ao_offsets,
                    dir_offsets,
                );
            }
        }
    }

    if data.transparent_count > 0 {
        for axis in 0..3usize {
            let axis_masks = &data.masks[axis];
            let first_dir = axis * 2;
            let second_dir = axis * 2 + 1;

            for c in 1..=REGION_SIZE {
                let emit_first = bitwise_and_not(
                    &axis_masks.rendered_non_full[c],
                    &axis_masks.full_cube[c - 1],
                );
                if !plane_is_zero(&emit_first) {
                    emit_plane_opaque::<true>(
                        &emit_first,
                        blocks,
                        full_mask,
                        tables,
                        ao_brightness,
                        builders,
                        c,
                        axis,
                        first_dir,
                        ao_offsets,
                        dir_offsets,
                    );
                }

                let emit_second = bitwise_and_not(
                    &axis_masks.rendered_non_full[c],
                    &axis_masks.full_cube[c + 1],
                );
                if !plane_is_zero(&emit_second) {
                    emit_plane_opaque::<true>(
                        &emit_second,
                        blocks,
                        full_mask,
                        tables,
                        ao_brightness,
                        builders,
                        c,
                        axis,
                        second_dir,
                        ao_offsets,
                        dir_offsets,
                    );
                }
            }
        }
    }
}

pub struct RegionChunkMeshBlocks {
    pub(crate) blocks: Box<[BlockType; PADDED_REGION_VOLUME]>,
    pub(crate) region_rendered_blocks: u32,
    pub(crate) region_full_cube_blocks: u32,
}

impl RegionChunkMeshBlocks {
    pub fn empty() -> Self {
        Self {
            blocks: Box::new([BlockType::Air; PADDED_REGION_VOLUME]),
            region_rendered_blocks: 0,
            region_full_cube_blocks: 0,
        }
    }

    pub fn from_chunks(region_min_chunk: IVec3, chunks: &HashMap<IVec3, &Chunk>) -> Self {
        let mut blocks = Self::empty();

        for dx in -1..=REGION_CHUNKS as i32 + 1 {
            for dy in -1..=REGION_CHUNKS as i32 + 1 {
                for dz in -1..=REGION_CHUNKS as i32 + 1 {
                    let chunk_pos = region_min_chunk + ivec3(dx, dy, dz);
                    if let Some(chunk) = chunks.get(&chunk_pos).copied() {
                        copy_chunk_into_region(&mut blocks, chunk_pos, chunk, region_min_chunk);
                    }
                }
            }
        }

        blocks
    }

    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block: BlockType) {
        debug_assert!(is_in_padded_region(x));
        debug_assert!(is_in_padded_region(y));
        debug_assert!(is_in_padded_region(z));

        let x = (x + 1) as usize;
        let y = (y + 1) as usize;
        let z = (z + 1) as usize;
        let idx = padded_region_index(x, y, z);
        self.blocks[idx] = block;
        self.region_rendered_blocks += block.is_rendered() as u32;
        self.region_full_cube_blocks += block.is_full_cube() as u32;
    }

    pub fn can_skip_mesh(&self) -> bool {
        self.region_rendered_blocks == 0
            || (self.region_is_all_full_cube() && self.neighbor_face_shells_are_full_cube())
    }

    pub(crate) fn region_is_all_full_cube(&self) -> bool {
        self.region_full_cube_blocks as usize == REGION_SIZE * REGION_SIZE * REGION_SIZE
    }

    fn neighbor_face_shells_are_full_cube(&self) -> bool {
        let r = PADDED_REGION_SIZE;
        for y in 1..=REGION_SIZE {
            for z in 1..=REGION_SIZE {
                if !self.blocks[padded_region_index(0, y, z)].is_full_cube()
                    || !self.blocks[padded_region_index(r - 1, y, z)].is_full_cube()
                {
                    return false;
                }
            }
        }
        for x in 1..=REGION_SIZE {
            for z in 1..=REGION_SIZE {
                if !self.blocks[padded_region_index(x, 0, z)].is_full_cube()
                    || !self.blocks[padded_region_index(x, r - 1, z)].is_full_cube()
                {
                    return false;
                }
            }
        }
        for x in 1..=REGION_SIZE {
            for y in 1..=REGION_SIZE {
                if !self.blocks[padded_region_index(x, y, 0)].is_full_cube()
                    || !self.blocks[padded_region_index(x, y, r - 1)].is_full_cube()
                {
                    return false;
                }
            }
        }
        true
    }
}

pub struct RegionMeshInput<'a> {
    pub blocks: &'a RegionChunkMeshBlocks,
    pub block_texture_map: &'a BlockTextureMap,
    pub ao_brightness: [f32; 4],
}

pub fn make_greedy_region_mesh(input: RegionMeshInput<'_>) -> ChunkLayerMeshes {
    if input.blocks.can_skip_mesh() {
        return Vec::new();
    }

    let tables = BlockMeshTables::from_texture_map(input.block_texture_map);
    let blocks = &input.blocks.blocks;

    let ao_offsets = make_ao_offsets();
    let dir_offsets = make_direction_offsets();

    let data = build_bitmasks(blocks, ao_offsets, dir_offsets);
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

fn is_in_padded_region(value: i32) -> bool {
    (-1..=REGION_SIZE as i32).contains(&value)
}

fn copy_chunk_into_region(
    blocks: &mut RegionChunkMeshBlocks,
    chunk_pos: IVec3,
    chunk: &Chunk,
    region_min: IVec3,
) {
    let base_x = (chunk_pos.x - region_min.x) * CHUNK_ISIZE;
    let base_y = (chunk_pos.y - region_min.y) * CHUNK_ISIZE;
    let base_z = (chunk_pos.z - region_min.z) * CHUNK_ISIZE;

    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let wx = base_x + x as i32;
                let wy = base_y + y as i32;
                let wz = base_z + z as i32;
                if is_in_padded_region(wx) && is_in_padded_region(wy) && is_in_padded_region(wz) {
                    blocks.set_block(wx, wy, wz, chunk.blocks[x][z][y]);
                }
            }
        }
    }
}
