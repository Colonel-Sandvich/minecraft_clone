//! Greedy meshing for chunk faces.
//!
//! ## Algorithm overview
//!
//! 1. **Bitplane construction** — scan 18³ padded blocks, setting bits in per-axis,
//!    per-layer bitplanes for full cubes and transparent blocks.
//! 2. **Face counting** — for each axis and layer, `full_cube[c] & !full_cube[c±1]`
//!    yields emit planes; popcount gives exact face counts for buffer pre-allocation.
//! 3. **Greedy quad emission** — for each emit plane, walk set bits left-to-right.
//!    From each un-consumed anchor face, extend right (same AO key, block type,
//!    visibility) then down, marking consumed cells as merged.  Emits one merged
//!    quad per anchor.
//! 4. **Two-pass layering**: opaque solid blocks first, then transparent (leaves,
//!    glass), which subtract full_cube neighbours so they don't emit into opaque
//!    blocks.
//!
//! ## Per-vertex AO
//!
//! Ambient occlusion uses 3 precomputed side-neighbour lookups per vertex via
//! `AO_SAMPLE_INDEX_OFFSETS` → `VERTEX_AO`. 4 × 2-bit values are packed into a
//! `u8` AO key for fast extension comparison (`face_ao_key_from_pi`).  The key is
//! recomputed on-the-fly for each candidate cell during extension loops.
//!
//! **Attempted optimisation (reverted):** precomputing all 24,576 AO keys into a
//! static array and passing it through the pipeline.  This was slower because:
//! - transparent emit planes contain border/padding bits that get clipped by the
//!   `1..=CHUNK_SIZE` guard;
//! - opaque planes compute AO for cells that will be consumed by merging;
//! - over 90% of transparent emit bits are suppressed by `should_emit_face_from_indices`
//!   after the plane is built.
//! The lazy approach naturally skips all three categories, making precomputation
//! a net loss.  AO computation is already ~4 ns per vertex — the 40 % profile
//! share reflects genuine unavoidable work.
//!
//! ## Known improvement opportunities
//!
//! - `merged_ao` re-evaluates the same 4 corner AOs that `face_ao_key_from_pi`
//!   already computed for the anchor — could reuse the anchor key.
//! - Extension loops re-check AO keys and transparency visibility per-cell;
//!   reducing merge attempts in transparency-dense regions (where most extensions
//!   fail) would help more than micro-optimising the AO computation itself.

use crate::block::BlockMaterialLayer;

use super::{
    AO_SAMPLE_INDEX_OFFSETS, BlockMeshTables, BlockType, CHUNK_SIZE, ChunkLayerMeshes,
    ChunkMeshInput, ChunkMesher, DIRECTION_INDEX_OFFSETS, MeshBufferBuilder,
    PADDED_CHUNK_LAYER_SIZE, PADDED_CHUNK_SIZE, PADDED_CHUNK_VOLUME, VERTEX_AO, padded_chunk_index,
    should_emit_face_from_indices,
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

const FULL_MASK_U64S: usize = PADDED_CHUNK_VOLUME.div_ceil(64);

#[derive(Clone, Copy)]
struct FullCubeMask([u64; FULL_MASK_U64S]);

impl FullCubeMask {
    #[inline(always)]
    #[allow(dead_code)]
    fn is_full_cube(&self, idx: usize) -> bool {
        (self.0[idx / 64] >> (idx % 64)) & 1 != 0
    }
}

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
    rendered_non_full: [BitPlane; PADDED_CHUNK_SIZE],
}

struct GreedyData {
    masks: [AxisMasks; 3],
    transparent_count: usize,
    full_mask: FullCubeMask,
}

/// Single-pass scan of 18³ padded blocks → per-axis bitplanes.
///
/// For each block position, if the block is a full cube we set its bit in all
/// three axis planes (the face-validity test is `full_cube[a] & !full_cube[a±1]`
/// later).  Rendered non-full blocks (leaves, glass) go into the transparent
/// planes instead, and we tally `transparent_count` for buffer sizing.
fn build_bitmasks(blocks: &[BlockType; PADDED_CHUNK_VOLUME]) -> GreedyData {
    let mut masks = std::array::from_fn(|_| AxisMasks {
        full_cube: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
        rendered_non_full: [[0u64; PLANE_U64S]; PADDED_CHUNK_SIZE],
    });
    let mut full_mask = [0u64; FULL_MASK_U64S];
    let mut transparent_count = 0;

    for py in 0..PADDED_CHUNK_SIZE {
        for pz in 0..PADDED_CHUNK_SIZE {
            let mut pi = padded_chunk_index(0, py, pz);
            for px in 0..PADDED_CHUNK_SIZE {
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
    }
}

#[inline(always)]
fn single_vertex_ao(
    full_mask: &FullCubeMask,
    padded_index: usize,
    side_index: usize,
    vertex_index: usize,
) -> u8 {
    let offsets = AO_SAMPLE_INDEX_OFFSETS[side_index][vertex_index];
    let m = &full_mask.0;
    let idx0 = (padded_index as isize + offsets[0]) as usize;
    let idx1 = (padded_index as isize + offsets[1]) as usize;
    let idx2 = (padded_index as isize + offsets[2]) as usize;
    let s1 = (m[idx0 >> 6] >> (idx0 & 63)) & 1;
    let s2 = (m[idx1 >> 6] >> (idx1 & 63)) & 1;
    let co = (m[idx2 >> 6] >> (idx2 & 63)) & 1;
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

/// Recompute AO for the 4 corners of a merged quad (width × height).
///
/// Maps each corner back to its owning block (the sub-cube at that corner's
/// position within the merged region), then calls `single_vertex_ao`.
///
/// NOTE: re-evaluates the same 4 `single_vertex_ao` calls that
/// `face_ao_key_from_pi` already did for the anchor face — possible reuse.
fn merged_ao(
    full_mask: &FullCubeMask,
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
        single_vertex_ao(full_mask, pi, side_index, vi)
    })
}

/// Count faces for buffer pre-allocation.
///
/// Opaque layer: for each axis/layer, `full_cube[c] & !full_cube[c±1]` yields
/// front/back emit planes; popcount gives exact count.
///
/// Transparent layer: worst-case `transparent_count * 6` (every transparent
/// block could emit all 6 faces).  Over-estimates when transparent blocks face
/// into opaque neighbours, but that's fine for capacity.
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

/// Pack 4 corner AO values (2 bits each) into a single `u8` for fast equality.
///
/// Equality of this key means all four vertex AO values match, which is
/// required for extending a quad in either direction.
#[inline(always)]
fn face_ao_key_from_pi(full_mask: &FullCubeMask, pi: usize, dir: usize) -> u8 {
    let a0 = single_vertex_ao(full_mask, pi, dir, 0);
    let a1 = single_vertex_ao(full_mask, pi, dir, 1);
    let a2 = single_vertex_ao(full_mask, pi, dir, 2);
    let a3 = single_vertex_ao(full_mask, pi, dir, 3);
    a0 | (a1 << 2) | (a2 << 4) | (a3 << 6)
}

#[inline(always)]
fn unpack_ao_key(key: u8) -> [u8; 4] {
    [key & 3, (key >> 2) & 3, (key >> 4) & 3, key >> 6]
}

/// Greedy quad emission for a single 16×16 emit bitplane.
///
/// Walks set bits left-to-right, top-to-bottom in the bitplane.  For each
/// un-consumed anchor face:
///
/// 1. Extend **right** while same block type, same AO key, face still set,
///    not consumed, and (for transparent) still emits to exterior.
/// 2. Extend **down** with the same checks for every column in the current
///    width; breaks entire row on first mismatch.
/// 3. Mark the `w × h` rectangle consumed.
/// 4. Emit one merged quad.
///
/// `IS_TRANSPARENT` controls whether `should_emit_face_from_indices` is
/// consulted (opaque blocks always emit when the bit is set).
fn emit_plane_opaque<const IS_TRANSPARENT: bool>(
    emit: &BitPlane,
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    full_mask: &FullCubeMask,
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

            // --- Anchor face ---
            let (pad, world) = match axis {
                0 => ([c, t1, t2], [c - 1, t1 - 1, t2 - 1]),
                1 => ([t1, c, t2], [t1 - 1, c - 1, t2 - 1]),
                _ => ([t1, t2, c], [t1 - 1, t2 - 1, c - 1]),
            };
            let pi = padded_chunk_index(pad[0], pad[1], pad[2]);
            let block = blocks[pi];

            if IS_TRANSPARENT {
                let exterior_pi = (pi as isize + DIRECTION_INDEX_OFFSETS[dir]) as usize;
                if !should_emit_face_from_indices(block, blocks[exterior_pi]) {
                    consumed[cw] |= 1 << cb;
                    continue;
                }
            }

            let base_ao_key = face_ao_key_from_pi(full_mask, pi, dir);

            let wx = world[0];
            let wy = world[1];
            let wz = world[2];

            // --- Horizontal extension (right) ---
            let row_start_bit = t1 * PADDED_CHUNK_SIZE + t2;
            let rw = row_start_bit / 64;
            let rb = row_start_bit % 64;
            let word_bits = emit[rw] >> rb;
            // trailing_ones includes the anchor bit; subtract 1, cap at CHUNK_SIZE and word boundary
            let max_emit_run = ((word_bits.trailing_ones() as usize) - 1)
                .min(CHUNK_SIZE - t2)
                .min(64 - rb - 1);

            let mut w_ext = 1;
            while w_ext <= max_emit_run {
                let p = (t1 - 1) * packed_stride + (t2 + w_ext - 1);
                if consumed[p / 64] & (1 << (p % 64)) != 0 {
                    break;
                }

                let np = pi + step_w * w_ext;
                if IS_TRANSPARENT {
                    let exterior_np = (np as isize + DIRECTION_INDEX_OFFSETS[dir]) as usize;
                    if !should_emit_face_from_indices(blocks[np], blocks[exterior_np]) {
                        break;
                    }
                }
                if face_ao_key_from_pi(full_mask, np, dir) != base_ao_key {
                    break;
                }
                if blocks[np] != block {
                    break;
                }
                w_ext += 1;
            }

            // --- Vertical extension (down) ---
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
                    if IS_TRANSPARENT {
                        let exterior_np = (np as isize + DIRECTION_INDEX_OFFSETS[dir]) as usize;
                        if !should_emit_face_from_indices(blocks[np], blocks[exterior_np]) {
                            break 'vloop;
                        }
                    }
                    if face_ao_key_from_pi(full_mask, np, dir) != base_ao_key {
                        break 'vloop;
                    }
                    if blocks[np] != block {
                        break 'vloop;
                    }
                }
                h_ext += 1;
            }

            // --- Mark merged region as consumed ---
            for dy in 0..h_ext {
                for dx in 0..w_ext {
                    let p = (t1 + dy - 1) * packed_stride + (t2 + dx - 1);
                    consumed[p / 64] |= 1 << (p % 64);
                }
            }

            // --- Emit merged quad ---
            let (emit_w, emit_h) = match axis {
                0 => (w_ext, h_ext),
                1 => (h_ext, w_ext),
                _ => (h_ext, w_ext),
            };

            let bi = block as usize;
            let ao = if w_ext == 1 && h_ext == 1 {
                unpack_ao_key(base_ao_key)
            } else {
                merged_ao(full_mask, dir, wx, wy, wz, emit_w, emit_h)
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

/// Main face-emission loop: opaque pass, then transparent pass.
///
/// For each axis and layer, builds front/back emit planes as
/// `full_cube[c] & !full_cube[c±1]` and delegates to `emit_plane_opaque`.
/// The transparent pass subtracts full_cube neighbours to avoid interior faces.
fn emit_faces(
    blocks: &[BlockType; PADDED_CHUNK_VOLUME],
    data: &GreedyData,
    tables: &BlockMeshTables,
    ao_brightness: [f32; 4],
    builders: &mut [MeshBufferBuilder; BlockMaterialLayer::COUNT],
) {
    let full_mask = &data.full_mask;

    for axis in 0..3usize {
        let axis_masks = &data.masks[axis];
        let first_dir = axis * 2;
        let second_dir = axis * 2 + 1;

        for c in 1..=CHUNK_SIZE {
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
                );
            }
        }
    }

    if data.transparent_count > 0 {
        for axis in 0..3usize {
            let axis_masks = &data.masks[axis];
            let first_dir = axis * 2;
            let second_dir = axis * 2 + 1;

            for c in 1..=CHUNK_SIZE {
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
                    );
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
