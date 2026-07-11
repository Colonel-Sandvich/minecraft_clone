//! Packed CPU/GPU face representation used by the terrain mesher.
//!
//! `PackedFace` is the narrow interface between mesh generation and the
//! vertex-pulling render backend. Keep this layout in sync with the shader's
//! two-word face record.

use crate::block::RENDER_ID_COUNT;
use crate::quad::Direction;

const WATER_FLOWING_MASK: u32 = 1 << 0;
const WATER_FLOW_CODE_SHIFT: u32 = 1;
const WATER_FLOW_CODE_MASK: u32 = 0xF << WATER_FLOW_CODE_SHIFT;
const WATER_GEOMETRY_MASK: u32 = 1 << 5;
const WATER_BELOW_LO_SHIFT: u32 = 6;
const WATER_BELOW_LO_MASK: u32 = 0xF << WATER_BELOW_LO_SHIFT;
const WATER_BELOW_HI_SHIFT: u32 = 10;
const WATER_BELOW_HI_MASK: u32 = 0xF << WATER_BELOW_HI_SHIFT;
const FACE_DIRECTION_SHIFT: u32 = 14;
const FACE_DIRECTION_MASK: u32 = 0x7 << FACE_DIRECTION_SHIFT;
const Y_SHIFT: u32 = 17;
const Y_MASK: u32 = 0x1F << Y_SHIFT;
const Z_SHIFT: u32 = 22;
const Z_MASK: u32 = 0x1F << Z_SHIFT;
const X_SHIFT: u32 = 27;
const X_MASK: u32 = 0x1F << X_SHIFT;

const RENDER_ID_SHIFT: u32 = 0;
const RENDER_ID_BITS: u32 = 8;
const RENDER_ID_MASK: u32 = ((1 << RENDER_ID_BITS) - 1) << RENDER_ID_SHIFT;
const AO_KEY_SHIFT: u32 = 8;
const AO_KEY_MASK: u32 = 0xFF << AO_KEY_SHIFT;
const WATER_CORNER_HEIGHTS_SHIFT: u32 = 16;
const WATER_CORNER_HEIGHTS_MASK: u32 = 0xFFFF << WATER_CORNER_HEIGHTS_SHIFT;

const _: () = assert!(RENDER_ID_COUNT <= (1 << RENDER_ID_BITS));
const _: () = assert!(Direction::COUNT <= (1 << 3));

/// One visible block face in the terrain shader's storage-buffer format.
///
/// The two words are intentionally private. Meshing code constructs faces
/// through the packing methods, while the renderer treats the value as an
/// opaque eight-byte ABI record.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PackedFace {
    packed: u32,
    info: u32,
}

const _: () = assert!(std::mem::size_of::<PackedFace>() == 8);
const _: () = assert!(std::mem::align_of::<PackedFace>() == 4);

impl PackedFace {
    #[inline]
    pub(crate) fn new(
        x: u32,
        y: u32,
        z: u32,
        face_direction: u32,
        render_id: u32,
        ao_key: u32,
    ) -> Self {
        debug_assert!(x <= 0x1F);
        debug_assert!(y <= 0x1F);
        debug_assert!(z <= 0x1F);
        debug_assert!((face_direction as usize) < Direction::COUNT);
        debug_assert!(render_id <= 0xFF);
        debug_assert!(ao_key <= 0xFF);

        Self {
            packed: ((x << X_SHIFT) & X_MASK)
                | ((z << Z_SHIFT) & Z_MASK)
                | ((y << Y_SHIFT) & Y_MASK)
                | ((face_direction << FACE_DIRECTION_SHIFT) & FACE_DIRECTION_MASK),
            info: ((render_id << RENDER_ID_SHIFT) & RENDER_ID_MASK)
                | ((ao_key << AO_KEY_SHIFT) & AO_KEY_MASK),
        }
    }

    /// Add the four top-surface heights, measured in ninths of a block.
    #[inline]
    pub(crate) fn with_corner_heights(mut self, h00: u32, h10: u32, h01: u32, h11: u32) -> Self {
        self.packed |= WATER_GEOMETRY_MASK;
        self.info |= (h00 & 0xF) << WATER_CORNER_HEIGHTS_SHIFT
            | (h10 & 0xF) << (WATER_CORNER_HEIGHTS_SHIFT + 4)
            | (h01 & 0xF) << (WATER_CORNER_HEIGHTS_SHIFT + 8)
            | (h11 & 0xF) << (WATER_CORNER_HEIGHTS_SHIFT + 12);
        self
    }

    /// Add the two lower-surface heights needed by a water side face.
    #[inline]
    pub(crate) fn with_water_below(mut self, lo: u32, hi: u32) -> Self {
        self.packed |= ((lo & 0xF) << WATER_BELOW_LO_SHIFT) | ((hi & 0xF) << WATER_BELOW_HI_SHIFT);
        self
    }

    /// Select the flowing-water texture and encode its horizontal direction.
    #[inline]
    pub(crate) fn with_water_up_flow(mut self, flow_code: u32) -> Self {
        self.packed |= WATER_FLOWING_MASK | ((flow_code & 0xF) << WATER_FLOW_CODE_SHIFT);
        self
    }

    #[inline]
    pub const fn x(self) -> u32 {
        (self.packed & X_MASK) >> X_SHIFT
    }

    #[inline]
    pub const fn y(self) -> u32 {
        (self.packed & Y_MASK) >> Y_SHIFT
    }

    #[inline]
    pub const fn z(self) -> u32 {
        (self.packed & Z_MASK) >> Z_SHIFT
    }

    #[inline]
    pub const fn face_direction(self) -> u32 {
        (self.packed & FACE_DIRECTION_MASK) >> FACE_DIRECTION_SHIFT
    }

    #[inline]
    pub const fn render_id(self) -> u32 {
        (self.info & RENDER_ID_MASK) >> RENDER_ID_SHIFT
    }

    #[inline]
    pub const fn ao_key(self) -> u32 {
        (self.info & AO_KEY_MASK) >> AO_KEY_SHIFT
    }

    #[inline]
    pub const fn water_up_flowing(self) -> bool {
        self.packed & WATER_FLOWING_MASK != 0
    }

    #[inline]
    pub const fn water_flow_code(self) -> u32 {
        (self.packed & WATER_FLOW_CODE_MASK) >> WATER_FLOW_CODE_SHIFT
    }

    #[inline]
    pub const fn has_water_geometry(self) -> bool {
        self.packed & WATER_GEOMETRY_MASK != 0
    }

    #[inline]
    pub const fn water_below(self) -> (u32, u32) {
        (
            (self.packed & WATER_BELOW_LO_MASK) >> WATER_BELOW_LO_SHIFT,
            (self.packed & WATER_BELOW_HI_MASK) >> WATER_BELOW_HI_SHIFT,
        )
    }

    #[inline]
    pub const fn water_corner_heights(self) -> (u32, u32, u32, u32) {
        let heights = (self.info & WATER_CORNER_HEIGHTS_MASK) >> WATER_CORNER_HEIGHTS_SHIFT;
        (
            heights & 0xF,
            (heights >> 4) & 0xF,
            (heights >> 8) & 0xF,
            (heights >> 12) & 0xF,
        )
    }

    #[inline]
    pub(crate) const fn words(self) -> [u32; 2] {
        [self.packed, self.info]
    }

    #[inline]
    pub(crate) fn apply_packed_bits(&mut self, packed: u32, info: u32) {
        self.packed |= packed;
        self.info |= info;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_face_has_shader_abi_layout() {
        assert_eq!(std::mem::size_of::<PackedFace>(), 8);
        assert_eq!(std::mem::align_of::<PackedFace>(), 4);
    }

    #[test]
    fn base_fields_round_trip_at_mask_boundaries() {
        let face = PackedFace::new(0x1F, 0x1F, 0x1F, 5, 0xFF, 0xFF);

        assert_eq!(face.x(), 0x1F);
        assert_eq!(face.y(), 0x1F);
        assert_eq!(face.z(), 0x1F);
        assert_eq!(face.face_direction(), 5);
        assert_eq!(face.render_id(), 0xFF);
        assert_eq!(face.ao_key(), 0xFF);
    }

    #[test]
    fn water_fields_round_trip_at_mask_boundaries() {
        let face = PackedFace::new(31, 30, 29, 5, 0xFE, 0xFD)
            .with_corner_heights(0xF, 0xE, 0xD, 0xC)
            .with_water_below(0xB, 0xA)
            .with_water_up_flow(0x9);

        assert!(face.has_water_geometry());
        assert!(face.water_up_flowing());
        assert_eq!(face.water_flow_code(), 0x9);
        assert_eq!(face.water_below(), (0xB, 0xA));
        assert_eq!(face.water_corner_heights(), (0xF, 0xE, 0xD, 0xC));
        assert_eq!((face.x(), face.y(), face.z()), (31, 30, 29));
        assert_eq!(face.face_direction(), 5);
        assert_eq!(face.render_id(), 0xFE);
        assert_eq!(face.ao_key(), 0xFD);
    }

    #[test]
    fn packed_word_fields_do_not_overlap() {
        let water_mask = WATER_FLOWING_MASK
            | WATER_FLOW_CODE_MASK
            | WATER_GEOMETRY_MASK
            | WATER_BELOW_LO_MASK
            | WATER_BELOW_HI_MASK;
        let geometry_mask = FACE_DIRECTION_MASK | X_MASK | Y_MASK | Z_MASK;
        assert_eq!(water_mask & geometry_mask, 0);
        assert_eq!(water_mask | geometry_mask, u32::MAX);

        assert_eq!(RENDER_ID_MASK & AO_KEY_MASK, 0);
        assert_eq!(RENDER_ID_MASK & WATER_CORNER_HEIGHTS_MASK, 0);
        assert_eq!(AO_KEY_MASK & WATER_CORNER_HEIGHTS_MASK, 0);
        assert_eq!(
            RENDER_ID_MASK | AO_KEY_MASK | WATER_CORNER_HEIGHTS_MASK,
            u32::MAX
        );
    }
}
