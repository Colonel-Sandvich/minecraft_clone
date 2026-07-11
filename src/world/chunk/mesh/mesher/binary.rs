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

use crate::block::{BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED, BlockMaterialLayer};

use super::{
    super::{
        blocks::{
            ChunkMeshBlocks, DIRECTION_COUNT, DIRECTION_INDEX_OFFSETS, PADDED_CHUNK_LAYER_SIZE,
            PADDED_CHUNK_SIZE, padded_chunk_index,
        },
        face::PackedFace,
    },
    LayerMesh,
    ao::{
        FACE_AO_ORDERS, FACE_AO_SAMPLE_COUNT, FACE_AO_SAMPLE_OFFSETS, face_ao_key_from_sample_bits,
    },
    face_capacity_estimate,
    visibility::block_mesh_flags,
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
    fn from_padded(blocks: &ChunkMeshBlocks) -> Self {
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

                for (z, z_row) in z_rows.iter_mut().enumerate() {
                    let idx = base + z * pad;
                    let flags = block_mesh_flags(unsafe { *cells.get_unchecked(idx) });
                    if flags & (BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE)
                        == BLOCK_FLAG_RENDERED | BLOCK_FLAG_FULL_CUBE
                    {
                        row |= 1u64 << z;
                        *z_row |= 1u64 << y;
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

const FACE_AO_MASK_OFFSETS: [[isize; FACE_AO_SAMPLE_COUNT]; DIRECTION_COUNT] = [
    face_ao_mask_offsets(0),
    face_ao_mask_offsets(1),
    face_ao_mask_offsets(2),
    face_ao_mask_offsets(3),
    face_ao_mask_offsets(4),
    face_ao_mask_offsets(5),
];

const fn face_ao_mask_offsets(side: usize) -> [isize; FACE_AO_SAMPLE_COUNT] {
    let mut offsets = [0; FACE_AO_SAMPLE_COUNT];
    let mut i = 0;
    while i < FACE_AO_SAMPLE_COUNT {
        offsets[i] = project_face_ao_offset(side, FACE_AO_SAMPLE_OFFSETS[side][i]);
        i += 1;
    }
    offsets
}

const fn project_face_ao_offset(side: usize, offset: isize) -> isize {
    let tangent = offset - DIRECTION_INDEX_OFFSETS[side];
    match side {
        0 | 1 => div_exact(tangent, PADDED_CHUNK_SIZE as isize),
        2 | 3 => {
            let (dx, dz) = split_minor_major_offset(tangent, PADDED_CHUNK_SIZE as isize);
            dx * PADDED_CHUNK_SIZE as isize + dz
        }
        4 | 5 => {
            let (dx, dy) = split_minor_major_offset(tangent, PADDED_CHUNK_LAYER_SIZE as isize);
            dx * PADDED_CHUNK_SIZE as isize + dy
        }
        _ => panic!("invalid face side"),
    }
}

const fn div_exact(value: isize, divisor: isize) -> isize {
    if value % divisor != 0 {
        panic!("invalid AO offset");
    }
    value / divisor
}

const fn split_minor_major_offset(tangent: isize, major_stride: isize) -> (isize, isize) {
    if tangent % major_stride == 0 {
        (0, tangent / major_stride)
    } else if (tangent - 1) % major_stride == 0 {
        (1, (tangent - 1) / major_stride)
    } else if (tangent + 1) % major_stride == 0 {
        (-1, (tangent + 1) / major_stride)
    } else {
        panic!("invalid AO offset");
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

    let [
        a0_off,
        a1_off,
        b0_off,
        b1_off,
        c00_off,
        c01_off,
        c10_off,
        c11_off,
    ] = FACE_AO_MASK_OFFSETS[side];

    #[inline(always)]
    fn ao_bit(plane: &PlaneBits, bit: usize) -> u32 {
        (plane.words[bit >> 6] >> (bit & 63)) as u32 & 1
    }

    let b = bit as isize;
    face_ao_key_from_sample_bits(
        FACE_AO_ORDERS[side],
        [
            ao_bit(mask, (b + a0_off) as usize),
            ao_bit(mask, (b + a1_off) as usize),
            ao_bit(mask, (b + b0_off) as usize),
            ao_bit(mask, (b + b1_off) as usize),
            ao_bit(mask, (b + c00_off) as usize),
            ao_bit(mask, (b + c01_off) as usize),
            ao_bit(mask, (b + c10_off) as usize),
            ao_bit(mask, (b + c11_off) as usize),
        ],
    )
}

/// Run binary full-cube face meshing and return the opaque layer.
///
/// Only full-cube blocks are handled here. The production entry point follows
/// this with the scalar shaped-block pass when the neighborhood needs it.
pub(super) fn build_binary(blocks: &ChunkMeshBlocks) -> Vec<LayerMesh> {
    if blocks.can_skip_mesh() {
        return Vec::new();
    }

    let mut faces = Vec::with_capacity(face_capacity_estimate(blocks.center_full_cube_blocks));
    push_binary_faces(blocks, &mut faces);

    if faces.is_empty() {
        Vec::new()
    } else {
        vec![LayerMesh {
            material_layer: BlockMaterialLayer::Opaque,
            faces,
        }]
    }
}

pub(super) fn push_binary_faces(blocks: &ChunkMeshBlocks, faces: &mut Vec<PackedFace>) {
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
            faces.push(PackedFace::new(
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
            faces.push(PackedFace::new(
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
            faces.push(PackedFace::new(
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
            faces.push(PackedFace::new(
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
            faces.push(PackedFace::new(
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
            faces.push(PackedFace::new(
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
/// real function — but skips AO computation, cell lookup, and packed-face
/// construction.  Returns the face count so the loop body isn't DCE'd.
///
/// Use this to measure how much time is left after eliminating the per-face
/// heavy operations.
pub(super) fn benchmark_binary_floor(blocks: &ChunkMeshBlocks) -> usize {
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

#[cfg(test)]
#[path = "binary/tests.rs"]
mod tests;
