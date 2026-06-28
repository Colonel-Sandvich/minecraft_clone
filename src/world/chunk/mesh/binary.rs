//! Binary face-mask meshing for full-cube blocks.
//!
//! Builds axis-aligned bitmasks from the padded 18³ buffer and uses bitwise
//! AND-NOT operations to cull occluded faces. Only full-cube face occluders
//! participate; non-full-cube blocks (glass, leaves, water, ice) still use
//! the scalar path via the existing `should_emit_face_from_flags`.
//!
//! Representation
//! --------------
//! For each axis we store 18 planes of 18×18 bits (324 bits → 6×u64 per plane).
//!   - X-axis planes are YZ slices at each x ∈ [0,18)
//!   - Y-axis planes are XZ slices at each y ∈ [0,18)
//!   - Z-axis planes are XY slices at each z ∈ [0,18)
//!
//! This lets us use trivial plane-level AND-NOT for the axis-aligned faces:
//!   left_faces  = rendered[x] & !occluder[x-1]
//!   right_faces = rendered[x] & !occluder[x+1]
//! and similarly for Y and Z.

use crate::block::{
    BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED, BLOCK_FLAG_TRANSLUCENT, BlockMaterialLayer,
    WATER_RENDER_ID,
};

use super::vertex_pulling::{FaceDescriptor, water_face_descriptor};
use super::{
    CHUNK_SIZE, DIRECTION_INDEX_OFFSETS, PADDED_CHUNK_SIZE, block_mesh_flags,
    face_ao_key_from_indices, material_layer_index_from_flags, padded_chunk_index,
    should_emit_face_from_flags, should_emit_translucent_face, vertex_ao_key,
};

/// A packed 18×18 bitmask stored across 6 u64 words (324 bits).
#[derive(Clone, Debug)]
struct PlaneBits {
    words: [u64; 6],
}

impl PlaneBits {
    const fn zeroed() -> Self {
        Self { words: [0; 6] }
    }

    #[inline(always)]
    #[allow(dead_code)]
    fn set(&mut self, bit: usize) {
        self.words[bit / 64] |= 1u64 << (bit % 64);
    }

    #[inline(always)]
    fn and_not(&self, other: &Self) -> Self {
        let mut result = Self::zeroed();
        let mut i = 0;
        while i < 6 {
            result.words[i] = self.words[i] & !other.words[i];
            i += 1;
        }
        result
    }

    fn iter_ones(&self) -> OnesIter<'_> {
        OnesIter {
            words: &self.words,
            word_idx: 0,
            word: self.words[0],
        }
    }
}

struct OnesIter<'a> {
    words: &'a [u64; 6],
    word_idx: usize,
    word: u64,
}

impl Iterator for OnesIter<'_> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        loop {
            if self.word != 0 {
                let tz = self.word.trailing_zeros() as usize;
                self.word &= self.word - 1;
                return Some(self.word_idx * 64 + tz);
            }
            self.word_idx += 1;
            if self.word_idx >= 6 {
                return None;
            }
            self.word = unsafe { *self.words.get_unchecked(self.word_idx) };
        }
    }
}

/// Axis-aligned face masks for the padded 18³ neighborhood.
struct BinaryFaceMasks {
    /// X-axis: `rendered_x[x]` and `occluder_x[x]` are YZ planes at x.
    /// Bit layout: bit = y * 18 + z
    rendered_x: [PlaneBits; PADDED_CHUNK_SIZE],
    occluder_x: [PlaneBits; PADDED_CHUNK_SIZE],

    /// Y-axis: `rendered_y[y]` and `occluder_y[y]` are XZ planes at y.
    /// Bit layout: bit = x * 18 + z
    rendered_y: [PlaneBits; PADDED_CHUNK_SIZE],
    occluder_y: [PlaneBits; PADDED_CHUNK_SIZE],

    /// Z-axis: `rendered_z[z]` and `occluder_z[z]` are XY planes at z.
    /// Bit layout: bit = x * 18 + y
    rendered_z: [PlaneBits; PADDED_CHUNK_SIZE],
    occluder_z: [PlaneBits; PADDED_CHUNK_SIZE],
}

impl BinaryFaceMasks {
    /// Build masks for full-cube blocks via row-mask accumulation.
    ///
    /// Scans the padded 18³ block array once in XYZ order, building 18-bit row
    /// masks across z and y.  Each row is OR'd into its destination plane
    /// word(s) once, replacing per-cell RMW with per-row RMW.
    ///
    /// Flushes  X rows at (x,y),  Y rows at (y,x), and  Z rows at (z,x).
    /// Z-rows are accumulated into a tiny 18-element buffer because y is the
    /// middle loop while z is innermost.
    fn from_padded(blocks: &super::ChunkMeshBlocks) -> Self {
        let cells = &blocks.blocks;
        let pad = PADDED_CHUNK_SIZE;
        let pad_last = pad - 1;
        const P: usize = PADDED_CHUNK_SIZE;
        let mut rendered_x: [PlaneBits; P] = std::array::from_fn(|_| PlaneBits::zeroed());
        let mut occluder_x: [PlaneBits; P] = std::array::from_fn(|_| PlaneBits::zeroed());
        let mut rendered_y: [PlaneBits; P] = std::array::from_fn(|_| PlaneBits::zeroed());
        let mut occluder_y: [PlaneBits; P] = std::array::from_fn(|_| PlaneBits::zeroed());
        let mut rendered_z: [PlaneBits; P] = std::array::from_fn(|_| PlaneBits::zeroed());
        let mut occluder_z: [PlaneBits; P] = std::array::from_fn(|_| PlaneBits::zeroed());

        let mut z_rows: [u64; P] = [0u64; P];

        for x in 0..pad {
            let cx = x >= 1 && x < pad_last;
            z_rows.fill(0);

            for y in 0..pad {
                let mut row: u64 = 0;
                let base = padded_chunk_index(x, y, 0);

                for z in 0..pad {
                    let idx = base + z * pad;
                    let flags = block_mesh_flags(unsafe { *cells.get_unchecked(idx) });
                    if flags & (BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE)
                        == BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE
                    {
                        row |= 1u64 << z;
                        z_rows[z] |= 1u64 << y;
                    }
                }

                if row == 0 {
                    continue;
                }

                let cy = y >= 1 && y < pad_last;

                // X-axis: plane x, word at offset y·18
                Self::flush_row_pair(
                    &mut occluder_x[x],
                    &mut rendered_x[x],
                    y * pad,
                    row,
                    cx && cy,
                );

                // Y-axis: plane y, word at offset x·18
                Self::flush_row_pair(
                    &mut occluder_y[y],
                    &mut rendered_y[y],
                    x * pad,
                    row,
                    cx && cy,
                );
            }

            // Flush Z-axis rows for this x (accumulated across y)
            for z in 0..pad {
                let row = z_rows[z];
                if row != 0 {
                    Self::flush_row_pair(
                        &mut occluder_z[z],
                        &mut rendered_z[z],
                        x * pad,
                        row,
                        cx && z >= 1 && z < pad_last,
                    );
                }
            }
        }

        Self {
            rendered_x,
            occluder_x,
            rendered_y,
            occluder_y,
            rendered_z,
            occluder_z,
        }
    }

    /// Write an 18-bit row mask into occluder/rendered plane words.
    ///
    /// The row is shifted left by `bit_off` bits.  If it crosses a u64
    /// boundary the high bits spill into `words[w+1]`.
    ///
    /// `row` already includes the shell cells (z / y = 0 and 17).
    /// When `is_center` is true the shell bits are masked away for
    /// the rendered plane.
    #[inline(always)]
    fn flush_row_pair(
        occluder: &mut PlaneBits,
        rendered: &mut PlaneBits,
        bit_off: usize,
        row: u64,
        is_center: bool,
    ) {
        const W: usize = 64;
        const SHELL_MASK: u64 = !(1u64 | (1u64 << (PADDED_CHUNK_SIZE - 1)));

        let w = bit_off / W;
        let s = bit_off % W;

        occluder.words[w] |= row << s;
        if s + PADDED_CHUNK_SIZE > W {
            occluder.words[w + 1] |= row >> (W - s);
        }

        if is_center {
            let center = row & SHELL_MASK;
            if center != 0 {
                rendered.words[w] |= center << s;
                if s + PADDED_CHUNK_SIZE > W {
                    rendered.words[w + 1] |= center >> (W - s);
                }
            }
        }
    }
}

/// AO key from pre-built occluder bitmasks, avoiding 8 array loads + 8 flag
/// lookups per visible face. Uses `bit = y*18+z` (X faces), `bit = x*18+z`
/// (Y faces), `bit = x*18+y` (Z faces) plus axis-specific neighbour offsets.
///
/// The occluder masks are already hot in L1 from the AND-NOT plane loop, so
/// each corner check is a cheap word load + shift/test instead of a random
/// array access + flag table lookup.
#[inline(always)]
fn face_ao_key_from_masks(
    masks: &BinaryFaceMasks,
    bit: usize,
    side: usize,
    x: usize,
    y: usize,
    z: usize,
) -> u32 {
    let mask: &PlaneBits = match side {
        0 => &masks.occluder_x[x - 1],
        1 => &masks.occluder_x[x + 1],
        2 => &masks.occluder_y[y - 1],
        3 => &masks.occluder_y[y + 1],
        4 => &masks.occluder_z[z - 1],
        _ => &masks.occluder_z[z + 1],
    };

    // 8 AO corner-sample offsets (dy·18+dz for X/Y faces, dx·18+dy for Z)
    let offsets: &[isize; 8] = match side {
        0 => &[-18, 18, 1, -1, -17, -19, 19, 17], // -X, ab
        1 => &[-18, 18, -1, 1, -19, -17, 17, 19], // +X, ab
        2 => &[-18, 18, 1, -1, -17, -19, 19, 17], // -Y, ba
        3 => &[-18, 18, 1, -1, -17, 17, 19, -19], // +Y, ab
        4 => &[-18, 18, -1, 1, -19, 19, 17, -17], // -Z, ba
        _ => &[-18, 18, -1, 1, 17, 19, -19, -17], // +Z, ba
    };

    let [a0, a1, b0, b1, c00, c01, c10, c11] = *offsets;

    #[inline(always)]
    fn ao_bit(plane: &PlaneBits, bit: usize) -> u32 {
        (plane.words[bit >> 6] >> (bit & 63)) as u32 & 1
    }

    let b = bit as isize;
    let a0 = ao_bit(mask, (b + a0) as usize);
    let a1 = ao_bit(mask, (b + a1) as usize);
    let b0 = ao_bit(mask, (b + b0) as usize);
    let b1 = ao_bit(mask, (b + b1) as usize);
    let c00 = ao_bit(mask, (b + c00) as usize);
    let c01 = ao_bit(mask, (b + c01) as usize);
    let c10 = ao_bit(mask, (b + c10) as usize);
    let c11 = ao_bit(mask, (b + c11) as usize);

    // `ab` packing (sides 0, 1, 3) vs `ba` packing (sides 2, 4, 5)
    if side == 2 || side == 4 || side == 5 {
        vertex_ao_key(a0, b0, c00)
            | (vertex_ao_key(a1, b0, c10) << 2)
            | (vertex_ao_key(a0, b1, c01) << 4)
            | (vertex_ao_key(a1, b1, c11) << 6)
    } else {
        vertex_ao_key(a0, b0, c00)
            | (vertex_ao_key(a0, b1, c01) << 2)
            | (vertex_ao_key(a1, b0, c10) << 4)
            | (vertex_ao_key(a1, b1, c11) << 6)
    }
}

/// Run binary full-cube face meshing and return descriptors.
///
/// Only full-cube blocks are handled by the binary path. Non-full-cube
/// rendered cells are returned as `None` for the caller to handle via the
/// scalar path.
pub fn build_descriptors_binary(
    blocks: &super::ChunkMeshBlocks,
) -> Vec<(BlockMaterialLayer, Vec<FaceDescriptor>)> {
    if blocks.can_skip_mesh() {
        return Vec::new();
    }

    let mut descriptors = Vec::with_capacity(blocks.center_rendered_blocks as usize);
    push_descriptors_binary(blocks, &mut descriptors);

    if descriptors.is_empty() {
        Vec::new()
    } else {
        vec![(BlockMaterialLayer::Opaque, descriptors)]
    }
}

fn push_descriptors_binary(blocks: &super::ChunkMeshBlocks, descriptors: &mut Vec<FaceDescriptor>) {
    let masks = BinaryFaceMasks::from_padded(blocks);

    // X-axis faces: Left (side 0) and Right (side 1)
    for x in 1..PADDED_CHUNK_SIZE - 1 {
        // Left faces: rendered at x, occluder at x-1
        let left = masks.rendered_x[x].and_not(&masks.occluder_x[x - 1]);
        for bit in left.iter_ones() {
            let y = bit / PADDED_CHUNK_SIZE;
            let z = bit % PADDED_CHUNK_SIZE;
            let lx = x as u32 - 1;
            let cell = blocks.blocks[padded_chunk_index(x, y, z)];
            let ao_key = face_ao_key_from_masks(&masks, bit, 0, x, y, z);
            descriptors.push(FaceDescriptor::new(
                lx,
                y as u32 - 1,
                z as u32 - 1,
                0,
                cell as u32,
                ao_key,
            ));
        }

        // Right faces: rendered at x, occluder at x+1
        let right = masks.rendered_x[x].and_not(&masks.occluder_x[x + 1]);
        for bit in right.iter_ones() {
            let y = bit / PADDED_CHUNK_SIZE;
            let z = bit % PADDED_CHUNK_SIZE;
            let lx = x as u32 - 1;
            let cell = blocks.blocks[padded_chunk_index(x, y, z)];
            let ao_key = face_ao_key_from_masks(&masks, bit, 1, x, y, z);
            descriptors.push(FaceDescriptor::new(
                lx,
                y as u32 - 1,
                z as u32 - 1,
                1,
                cell as u32,
                ao_key,
            ));
        }
    }

    // Y-axis faces: Down (side 2) and Up (side 3)
    for y in 1..PADDED_CHUNK_SIZE - 1 {
        let down = masks.rendered_y[y].and_not(&masks.occluder_y[y - 1]);
        for bit in down.iter_ones() {
            let x = bit / PADDED_CHUNK_SIZE;
            let z = bit % PADDED_CHUNK_SIZE;
            let ly = y as u32 - 1;
            let cell = blocks.blocks[padded_chunk_index(x, y, z)];
            let ao_key = face_ao_key_from_masks(&masks, bit, 2, x, y, z);
            descriptors.push(FaceDescriptor::new(
                x as u32 - 1,
                ly,
                z as u32 - 1,
                2,
                cell as u32,
                ao_key,
            ));
        }

        let up = masks.rendered_y[y].and_not(&masks.occluder_y[y + 1]);
        for bit in up.iter_ones() {
            let x = bit / PADDED_CHUNK_SIZE;
            let z = bit % PADDED_CHUNK_SIZE;
            let ly = y as u32 - 1;
            let cell = blocks.blocks[padded_chunk_index(x, y, z)];
            let ao_key = face_ao_key_from_masks(&masks, bit, 3, x, y, z);
            descriptors.push(FaceDescriptor::new(
                x as u32 - 1,
                ly,
                z as u32 - 1,
                3,
                cell as u32,
                ao_key,
            ));
        }
    }

    // Z-axis faces: Front (side 4) and Back (side 5)
    for z in 1..PADDED_CHUNK_SIZE - 1 {
        let front = masks.rendered_z[z].and_not(&masks.occluder_z[z - 1]);
        for bit in front.iter_ones() {
            let x = bit / PADDED_CHUNK_SIZE;
            let y = bit % PADDED_CHUNK_SIZE;
            let lz = z as u32 - 1;
            let cell = blocks.blocks[padded_chunk_index(x, y, z)];
            let ao_key = face_ao_key_from_masks(&masks, bit, 4, x, y, z);
            descriptors.push(FaceDescriptor::new(
                x as u32 - 1,
                y as u32 - 1,
                lz,
                4,
                cell as u32,
                ao_key,
            ));
        }

        let back = masks.rendered_z[z].and_not(&masks.occluder_z[z + 1]);
        for bit in back.iter_ones() {
            let x = bit / PADDED_CHUNK_SIZE;
            let y = bit % PADDED_CHUNK_SIZE;
            let lz = z as u32 - 1;
            let cell = blocks.blocks[padded_chunk_index(x, y, z)];
            let ao_key = face_ao_key_from_masks(&masks, bit, 5, x, y, z);
            descriptors.push(FaceDescriptor::new(
                x as u32 - 1,
                y as u32 - 1,
                lz,
                5,
                cell as u32,
                ao_key,
            ));
        }
    }
}

/// Absolute floor for the binary path: builds masks, runs AND-NOT culling,
/// and iterates every surviving bit — the same memory access pattern as the
/// real function — but skips AO computation, cell lookup, and descriptor
/// construction.  Returns the face count so the loop body isn't DCE'd.
///
/// Use this to measure how much time is left after eliminating the per-face
/// heavy operations.
pub fn build_descriptors_binary_floor(blocks: &super::ChunkMeshBlocks) -> usize {
    if blocks.can_skip_mesh() {
        return 0;
    }

    let masks = BinaryFaceMasks::from_padded(blocks);
    let mut total = 0usize;

    // X-axis
    for x in 1..PADDED_CHUNK_SIZE - 1 {
        let left = masks.rendered_x[x].and_not(&masks.occluder_x[x - 1]);
        for bit in left.iter_ones() {
            let y = bit / PADDED_CHUNK_SIZE;
            let _z = bit % PADDED_CHUNK_SIZE;
            let _lx = x as u32 - 1;
            std::hint::black_box((y, _z, _lx));
            total += 1;
        }
        let right = masks.rendered_x[x].and_not(&masks.occluder_x[x + 1]);
        for bit in right.iter_ones() {
            let y = bit / PADDED_CHUNK_SIZE;
            let _z = bit % PADDED_CHUNK_SIZE;
            let _lx = x as u32 - 1;
            std::hint::black_box((y, _z, _lx));
            total += 1;
        }
    }

    // Y-axis
    for y in 1..PADDED_CHUNK_SIZE - 1 {
        let down = masks.rendered_y[y].and_not(&masks.occluder_y[y - 1]);
        for bit in down.iter_ones() {
            let x = bit / PADDED_CHUNK_SIZE;
            let _z = bit % PADDED_CHUNK_SIZE;
            let _ly = y as u32 - 1;
            std::hint::black_box((x, _z, _ly));
            total += 1;
        }
        let up = masks.rendered_y[y].and_not(&masks.occluder_y[y + 1]);
        for bit in up.iter_ones() {
            let x = bit / PADDED_CHUNK_SIZE;
            let _z = bit % PADDED_CHUNK_SIZE;
            let _ly = y as u32 - 1;
            std::hint::black_box((x, _z, _ly));
            total += 1;
        }
    }

    // Z-axis
    for z in 1..PADDED_CHUNK_SIZE - 1 {
        let front = masks.rendered_z[z].and_not(&masks.occluder_z[z - 1]);
        for bit in front.iter_ones() {
            let x = bit / PADDED_CHUNK_SIZE;
            let y = bit % PADDED_CHUNK_SIZE;
            let _lz = z as u32 - 1;
            std::hint::black_box((x, y, _lz));
            total += 1;
        }
        let back = masks.rendered_z[z].and_not(&masks.occluder_z[z + 1]);
        for bit in back.iter_ones() {
            let x = bit / PADDED_CHUNK_SIZE;
            let y = bit % PADDED_CHUNK_SIZE;
            let _lz = z as u32 - 1;
            std::hint::black_box((x, y, _lz));
            total += 1;
        }
    }

    total
}

/// Scalar pass for non-full-cube rendered blocks.
///
/// Iterates the 16³ center cells and emits faces only for blocks that are
/// rendered but NOT full-cube (glass, leaves, water, ice). Full-cube cells
/// are skipped here because `build_descriptors_binary` handles them.
fn push_descriptors_non_full_cube(
    blocks: &super::ChunkMeshBlocks,
    descriptors: &mut [Vec<FaceDescriptor>; BlockMaterialLayer::COUNT],
) {
    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let mut padded_index = padded_chunk_index(1, y + 1, z + 1);

            for x in 0..CHUNK_SIZE {
                let block = unsafe { *blocks.blocks.get_unchecked(padded_index) };
                let block_flags = block_mesh_flags(block);

                // Skip empty and full-cube cells (handled by binary pass)
                if block_flags == 0 || block_flags & BLOCK_FLAG_FULL_CUBE != 0 {
                    padded_index += 1;
                    continue;
                }

                let is_water = block == WATER_RENDER_ID;

                for (side_index, offset) in DIRECTION_INDEX_OFFSETS.iter().copied().enumerate() {
                    let neighbor_index = (padded_index as isize + offset) as usize;
                    let neighbor = unsafe { *blocks.blocks.get_unchecked(neighbor_index) };
                    let neighbor_flags = block_mesh_flags(neighbor);

                    let emit = if block_flags & BLOCK_FLAG_TRANSLUCENT != 0 {
                        should_emit_translucent_face(block, block_flags, neighbor, neighbor_flags)
                    } else {
                        should_emit_face_from_flags(block, block_flags, neighbor, neighbor_flags)
                    };

                    if emit {
                        let ao_key = face_ao_key_from_indices(blocks, padded_index, side_index);
                        let desc = FaceDescriptor::new(
                            x as u32,
                            y as u32,
                            z as u32,
                            side_index as u32,
                            block as u32,
                            ao_key,
                        );
                        descriptors[material_layer_index_from_flags(block_flags)].push(
                            if is_water {
                                water_face_descriptor(desc, blocks, padded_index, side_index)
                            } else {
                                desc
                            },
                        );
                    }
                }

                padded_index += 1;
            }
        }
    }
}

/// Hybrid meshing: binary face masks for full-cube blocks, scalar pass for
/// everything else (glass, leaves, water, ice). Descriptors are merged by
/// material layer.
///
/// This produces the same face count as the pure scalar `build_descriptors`
/// but uses the faster binary path for full-cube terrain.
pub fn build_descriptors_hybrid(
    blocks: &super::ChunkMeshBlocks,
) -> Vec<(BlockMaterialLayer, Vec<FaceDescriptor>)> {
    if blocks.can_skip_mesh() {
        return Vec::new();
    }

    if !blocks.has_non_full_cube_rendered() {
        return build_descriptors_binary(blocks);
    }

    let mut descriptors: [Vec<FaceDescriptor>; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|_| Vec::with_capacity(blocks.center_rendered_blocks as usize));
    push_descriptors_binary(blocks, &mut descriptors[BlockMaterialLayer::Opaque.index()]);
    push_descriptors_non_full_cube(blocks, &mut descriptors);

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            let layer_descriptors = std::mem::take(&mut descriptors[layer.index()]);
            (!layer_descriptors.is_empty()).then_some((layer, layer_descriptors))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{
        BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED, BlockType, WATER_RENDER_ID, render_id_for_block,
    };
    use crate::world::chunk::mesh::{
        CHUNK_SIZE, ChunkMeshBlocks, PADDED_CHUNK_VOLUME, block_mesh_flags, padded_chunk_index,
    };

    fn make_padded(kinds: &[u16]) -> ChunkMeshBlocks {
        let mut blocks = Box::new([0u16; PADDED_CHUNK_VOLUME]);
        let mut fluid_levels = Box::new([0u8; PADDED_CHUNK_VOLUME]);
        blocks[..kinds.len()].copy_from_slice(kinds);
        for (i, &kind) in kinds.iter().enumerate() {
            if kind == WATER_RENDER_ID {
                fluid_levels[i] = 8;
            }
        }
        let mut center_rendered = 0u16;
        let mut center_full = 0u16;
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    let idx = padded_chunk_index(x + 1, y + 1, z + 1);
                    let flags = block_mesh_flags(blocks[idx]);
                    if flags & BLOCK_FLAG_RENDERED != 0 {
                        center_rendered += 1;
                    }
                    if flags & BLOCK_FLAG_FULL_CUBE != 0 {
                        center_full += 1;
                    }
                }
            }
        }
        let mut result = ChunkMeshBlocks {
            blocks,
            fluid_levels,
            center_rendered_blocks: center_rendered,
            center_full_cube_blocks: center_full,
            neighbor_face_shells_full_cube: false,
            full_cube_cells: Vec::new().into_boxed_slice(),
        };
        result.compute_full_cube_cells();
        result
    }

    #[test]
    fn plane_bits_set_and_iterate() {
        let mut pb = PlaneBits::zeroed();
        pb.set(0);
        pb.set(5);
        pb.set(63);
        pb.set(64);
        pb.set(323);
        let ones: Vec<_> = pb.iter_ones().collect();
        assert_eq!(ones, vec![0, 5, 63, 64, 323]);
    }

    #[test]
    fn and_not_clears_matching_bits() {
        let mut a = PlaneBits::zeroed();
        let mut b = PlaneBits::zeroed();
        a.set(10);
        a.set(20);
        b.set(10);
        b.set(30);
        let result = a.and_not(&b);
        let ones: Vec<_> = result.iter_ones().collect();
        assert_eq!(ones, vec![20]);
    }

    #[test]
    fn binary_empty_chunk() {
        let padded = make_padded(&[0u16; PADDED_CHUNK_VOLUME]);
        let result = build_descriptors_binary(&padded);
        assert!(result.is_empty());
    }

    #[test]
    fn binary_single_full_cube_emits_six_faces() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        let center = padded_chunk_index(9, 9, 9);
        kinds[center] = render_id_for_block(BlockType::Stone);
        let padded = make_padded(&kinds);
        let result = build_descriptors_binary(&padded);
        let total: usize = result.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(total, 6, "single stone should emit 6 faces");
    }

    #[test]
    fn binary_two_adjacent_full_cubes_emit_ten_faces() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        let a = padded_chunk_index(9, 9, 9); // center
        let b = padded_chunk_index(10, 9, 9); // +X neighbor
        kinds[a] = render_id_for_block(BlockType::Stone);
        kinds[b] = render_id_for_block(BlockType::Stone);
        let padded = make_padded(&kinds);
        let result = build_descriptors_binary(&padded);
        let total: usize = result.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(total, 10, "two adjacent stones share 2 faces → 10 faces");
    }

    #[test]
    fn binary_full_cube_buried_emits_nothing() {
        let kinds = [render_id_for_block(BlockType::Stone); PADDED_CHUNK_VOLUME];
        let padded = make_padded(&kinds);
        let result = build_descriptors_binary(&padded);
        let total: usize = result.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(total, 0, "fully buried full-cube chunk emits 0 faces");
    }

    #[test]
    fn binary_stone_next_to_glass_emits_six_faces() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        let stone = padded_chunk_index(9, 9, 9);
        let glass = padded_chunk_index(10, 9, 9); // +X neighbor
        kinds[stone] = render_id_for_block(BlockType::Stone);
        kinds[glass] = render_id_for_block(BlockType::Glass); // Glass is rendered but NOT full_cube
        let padded = make_padded(&kinds);
        let result = build_descriptors_binary(&padded);
        let total: usize = result.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(
            total, 6,
            "binary path only handles full-cube; stone emits 6 faces"
        );
    }

    #[test]
    fn hybrid_stone_plus_glass_matches_scalar() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        let stone = padded_chunk_index(9, 9, 9);
        let glass = padded_chunk_index(10, 9, 9); // +X neighbor
        kinds[stone] = render_id_for_block(BlockType::Stone);
        kinds[glass] = render_id_for_block(BlockType::Glass);
        let padded = make_padded(&kinds);

        let scalar = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
        let hybrid = build_descriptors_hybrid(&padded);

        let scalar_total: usize = scalar.iter().map(|(_, d)| d.len()).sum();
        let hybrid_total: usize = hybrid.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(
            hybrid_total, scalar_total,
            "hybrid must match scalar face count"
        );
    }

    #[test]
    fn hybrid_water_stone_matches_scalar() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        let water = padded_chunk_index(9, 9, 9);
        let stone = padded_chunk_index(10, 9, 9);
        kinds[water] = WATER_RENDER_ID;
        kinds[stone] = render_id_for_block(BlockType::Stone);
        let padded = make_padded(&kinds);

        let scalar = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
        let hybrid = build_descriptors_hybrid(&padded);

        let scalar_total: usize = scalar.iter().map(|(_, d)| d.len()).sum();
        let hybrid_total: usize = hybrid.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(
            hybrid_total, scalar_total,
            "hybrid water+stone must match scalar"
        );
    }

    // This function compares hybrid output against scalar for correctness.
    // Individual sub-scenarios are tested inline with separate assert_eq calls
    // so failure messages identify the failing sub-scenario.
    #[test]
    fn hybrid_full_scenarios_match_scalar() {
        // empty
        {
            let padded = make_padded(&[0u16; PADDED_CHUNK_VOLUME]);
            let scalar = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
            let hybrid = build_descriptors_hybrid(&padded);
            assert_eq!(
                scalar.iter().map(|(_, d)| d.len()).sum::<usize>(),
                hybrid.iter().map(|(_, d)| d.len()).sum::<usize>(),
                "empty"
            );
        }
        // all stone
        {
            let padded = make_padded(&[render_id_for_block(BlockType::Stone); PADDED_CHUNK_VOLUME]);
            let scalar = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
            let hybrid = build_descriptors_hybrid(&padded);
            assert_eq!(
                scalar.iter().map(|(_, d)| d.len()).sum::<usize>(),
                hybrid.iter().map(|(_, d)| d.len()).sum::<usize>(),
                "all stone buried"
            );
        }
        // checkerboard
        {
            let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
            for x in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    for z in 0..CHUNK_SIZE {
                        if (x + y + z) % 2 == 0 {
                            kinds[padded_chunk_index(x + 1, y + 1, z + 1)] =
                                render_id_for_block(BlockType::Stone);
                        }
                    }
                }
            }
            let padded = make_padded(&kinds);
            let scalar = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
            let hybrid = build_descriptors_hybrid(&padded);
            assert_eq!(
                scalar.iter().map(|(_, d)| d.len()).sum::<usize>(),
                hybrid.iter().map(|(_, d)| d.len()).sum::<usize>(),
                "checkerboard"
            );
        }
        // stone + glass mixed
        {
            let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
            kinds[padded_chunk_index(9, 9, 9)] = render_id_for_block(BlockType::Stone);
            kinds[padded_chunk_index(9, 9, 8)] = render_id_for_block(BlockType::Stone);
            kinds[padded_chunk_index(9, 9, 10)] = render_id_for_block(BlockType::Glass);
            kinds[padded_chunk_index(9, 8, 9)] = render_id_for_block(BlockType::OakLeaves);
            kinds[padded_chunk_index(9, 10, 9)] = WATER_RENDER_ID;
            let padded = make_padded(&kinds);
            let scalar = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
            let hybrid = build_descriptors_hybrid(&padded);
            assert_eq!(
                scalar.iter().map(|(_, d)| d.len()).sum::<usize>(),
                hybrid.iter().map(|(_, d)| d.len()).sum::<usize>(),
                "mixed block types"
            );
        }
    }

    #[test]
    fn translucent_water_culled_by_stone() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        let water = padded_chunk_index(9, 9, 9);
        let stone = padded_chunk_index(10, 9, 9);
        kinds[water] = WATER_RENDER_ID;
        kinds[stone] = render_id_for_block(BlockType::Stone);
        let padded = make_padded(&kinds);

        let result = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
        let total: usize = result.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(total, 11, "water+stone: 6 stone + 5 water faces");
    }

    #[test]
    fn translucent_water_basin() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        kinds[padded_chunk_index(9, 9, 9)] = WATER_RENDER_ID;
        kinds[padded_chunk_index(9, 8, 9)] = render_id_for_block(BlockType::Stone); // below
        kinds[padded_chunk_index(10, 9, 9)] = render_id_for_block(BlockType::Stone); // +X
        kinds[padded_chunk_index(8, 9, 9)] = render_id_for_block(BlockType::Stone); // -X
        kinds[padded_chunk_index(9, 9, 10)] = render_id_for_block(BlockType::Stone); // +Z
        kinds[padded_chunk_index(9, 9, 8)] = render_id_for_block(BlockType::Stone); // -Z
        let padded = make_padded(&kinds);

        let result = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
        let total: usize = result.iter().map(|(_, d)| d.len()).sum();
        // Water: 1 up face. Five stone blocks: 6 faces each (water doesn't occlude).
        // Total = 1 + 6*5 = 31
        assert_eq!(total, 31, "water basin: 1 water up + 5*6 stone faces");
    }

    #[test]
    fn translucent_ice_culled_by_stone() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        kinds[padded_chunk_index(9, 9, 9)] = render_id_for_block(BlockType::Ice);
        kinds[padded_chunk_index(9, 8, 9)] = render_id_for_block(BlockType::Stone);
        let padded = make_padded(&kinds);

        let result = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
        let total: usize = result.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(total, 11, "ice+stone: 6 stone + 5 ice faces");
    }

    #[test]
    fn translucent_water_adjacent_water_culled() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        kinds[padded_chunk_index(9, 9, 9)] = WATER_RENDER_ID;
        kinds[padded_chunk_index(9, 9, 10)] = WATER_RENDER_ID;
        let padded = make_padded(&kinds);

        let result = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
        let total: usize = result.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(total, 10, "adjacent water: 10 faces (2 culled)");
    }

    #[test]
    fn water_connectivity_different_levels() {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        let idx_a = padded_chunk_index(9, 9, 9);
        let idx_b = padded_chunk_index(10, 9, 9);
        kinds[idx_a] = WATER_RENDER_ID;
        kinds[idx_b] = WATER_RENDER_ID;
        let mut padded = make_padded(&kinds);
        padded.fluid_levels[idx_a] = 5;
        padded.fluid_levels[idx_b] = 3;

        let scalar = crate::world::chunk::mesh::vertex_pulling::build_descriptors(&padded);
        let hybrid = build_descriptors_hybrid(&padded);
        let scalar_total: usize = scalar.iter().map(|(_, d)| d.len()).sum();
        let hybrid_total: usize = hybrid.iter().map(|(_, d)| d.len()).sum();
        assert_eq!(
            hybrid_total, scalar_total,
            "different water levels: hybrid matches scalar"
        );
        // A (level 5): 5 faces (water-water side culled, top slopes via corner heights)
        // B (level 3): 5 faces (same)
        // Total = 10
        assert_eq!(
            scalar_total, 10,
            "different water levels: 10 faces (top slopes)"
        );
    }

    // Recreate the realistic terrain used in the benchmark.
    // Inline `make_padded` call to avoid depending on tests-only helper.
    #[allow(clippy::if_same_then_else, clippy::manual_range_contains)]
    fn make_realistic_padded() -> ChunkMeshBlocks {
        let mut kinds = [0u16; PADDED_CHUNK_VOLUME];
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let idx = padded_chunk_index(x + 1, y + 1, z + 1);
                    kinds[idx] = if y < 4 {
                        render_id_for_block(BlockType::Stone)
                    } else if y < 6 {
                        render_id_for_block(BlockType::Dirt)
                    } else if y == 6 {
                        render_id_for_block(BlockType::Grass)
                    } else if y >= 7 && y <= 11 && x >= 6 && x <= 9 && z >= 6 && z <= 9 {
                        render_id_for_block(BlockType::OakLog)
                    } else if y == 12
                        && x >= 5
                        && x <= 10
                        && z >= 5
                        && z <= 10
                        && !(x >= 7 && x <= 8 && z >= 7 && z <= 8)
                    {
                        render_id_for_block(BlockType::OakLeaves)
                    } else if y == 11 && x >= 5 && x <= 10 && z >= 5 && z <= 10 {
                        render_id_for_block(BlockType::OakLeaves)
                    } else if y == 10
                        && x >= 5
                        && x <= 10
                        && z >= 5
                        && z <= 10
                        && (x == 5 || x == 10 || z == 5 || z == 10)
                    {
                        render_id_for_block(BlockType::OakLeaves)
                    } else if y == 3 && (x + z) % 13 == 0 {
                        render_id_for_block(BlockType::Glass)
                    } else if y == 8 && (x * 7 + z * 11) % 23 == 0 {
                        render_id_for_block(BlockType::Glass)
                    } else {
                        0u16
                    };
                }
            }
        }
        make_padded(&kinds)
    }

    #[test]
    fn perf_breakdown_binary() {
        use std::time::Instant;

        let blocks = make_realistic_padded();
        const ITERS: u64 = 50_000;

        // measure mask construction
        let t0 = Instant::now();
        for _ in 0..ITERS {
            let masks = BinaryFaceMasks::from_padded(&blocks);
            std::hint::black_box(masks);
        }
        let mask_elapsed = t0.elapsed();

        // measure full binary meshing
        let t0 = Instant::now();
        for _ in 0..ITERS {
            let result = build_descriptors_binary(&blocks);
            std::hint::black_box(result);
        }
        let full_elapsed = t0.elapsed();

        let mask_ns = mask_elapsed.as_nanos() as f64 / ITERS as f64;
        let full_ns = full_elapsed.as_nanos() as f64 / ITERS as f64;
        let rest_ns = full_ns - mask_ns;

        let face_count = build_descriptors_binary(&blocks)
            .iter()
            .map(|(_, d)| d.len())
            .sum::<usize>();

        println!();
        println!(
            "=== binary mesh breakdown (realistic, {} faces) ===",
            face_count
        );
        println!(
            "  mask build:           {:>9.0} ns  ({:.1}%)",
            mask_ns,
            mask_ns / full_ns * 100.0
        );
        println!(
            "  cull+emit+collect:    {:>9.0} ns  ({:.1}%)",
            rest_ns,
            rest_ns / full_ns * 100.0
        );
        println!("  total:                {:>9.0} ns", full_ns);
        println!(
            "  per-face:             {:>9.0} ns",
            full_ns / face_count as f64
        );
        println!();
    }
}
