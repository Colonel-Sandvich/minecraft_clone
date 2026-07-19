mod properties;
mod visual;

use crate::item::Item;

pub use properties::{
    BLOCK_FLAG_CUTOUT, BLOCK_FLAG_EMITS_INTERNAL_FACES, BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED,
    BLOCK_FLAG_TRANSLUCENT, BlockMaterialLayer, BlockRenderLayer, BlockRenderProfile,
    FaceOcclusion,
};
pub use visual::{
    BlockTextureAnimation, BlockTextureLayer, BlockTextureMap, BlockVisualTable,
    pack_texture_layer, render_id_to_colour, render_id_to_texture_path,
};

pub const fn render_id_for_block(block: Item) -> u16 {
    match block.block_storage_id() {
        Some(id) => id + 1,
        None => panic!("non-block item has no terrain render ID"),
    }
}

pub fn from_render_id(rid: u16) -> Option<Item> {
    Item::from_block_storage_id(rid.checked_sub(1)?)
}

pub const WATER_RENDER_ID: u16 = Item::BLOCK_COUNT as u16 + 1;
pub const RENDER_ID_COUNT: usize = Item::BLOCK_COUNT + 2;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_render_ids_roundtrip_and_exclude_reserved_ids() {
        for item in Item::BLOCKS {
            assert_eq!(from_render_id(render_id_for_block(item)), Some(item));
        }
        assert_eq!(from_render_id(0), None);
        assert_eq!(from_render_id(WATER_RENDER_ID), None);
    }
}
