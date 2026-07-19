//! Block classification and face-visibility rules shared by meshers.

use crate::block::{
    BLOCK_FLAG_CUTOUT, BLOCK_FLAG_EMITS_INTERNAL_FACES, BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED,
    BLOCK_FLAG_TRANSLUCENT, BlockMaterialLayer, WATER_RENDER_ID, from_render_id,
};

#[inline(always)]
pub(crate) fn block_mesh_flags(render_id: u16) -> u8 {
    match render_id {
        0 => 0,
        WATER_RENDER_ID => BLOCK_FLAG_RENDERED | BLOCK_FLAG_TRANSLUCENT,
        _ => from_render_id(render_id).unwrap().mesh_flags(),
    }
}

#[inline(always)]
pub(crate) const fn material_layer_index_from_flags(flags: u8) -> usize {
    if flags & BLOCK_FLAG_TRANSLUCENT != 0 {
        BlockMaterialLayer::Translucent.index()
    } else if flags & BLOCK_FLAG_CUTOUT != 0 {
        BlockMaterialLayer::Cutout.index()
    } else {
        BlockMaterialLayer::Opaque.index()
    }
}

#[inline(always)]
pub(crate) fn should_emit_face_from_flags(
    cell: u16,
    block_flags: u8,
    neighbor: u16,
    neighbor_flags: u8,
) -> bool {
    if neighbor_flags & BLOCK_FLAG_RENDERED == 0 {
        return true;
    }

    if neighbor_flags & BLOCK_FLAG_FULL_CUBE != 0 && block_flags & BLOCK_FLAG_TRANSLUCENT == 0 {
        return false;
    }

    if cell == neighbor && block_flags & BLOCK_FLAG_FULL_CUBE == 0 {
        return block_flags & BLOCK_FLAG_EMITS_INTERNAL_FACES != 0;
    }

    true
}

/// Translucent blocks are hidden by full-cube occluders and by another block
/// of the same fluid/material render ID.
#[inline(always)]
pub(crate) fn should_emit_translucent_face(
    cell: u16,
    _block_flags: u8,
    neighbor: u16,
    neighbor_flags: u8,
) -> bool {
    if neighbor_flags & BLOCK_FLAG_RENDERED == 0 {
        return true;
    }

    if neighbor_flags & BLOCK_FLAG_FULL_CUBE != 0 {
        return false;
    }

    cell != neighbor
}
