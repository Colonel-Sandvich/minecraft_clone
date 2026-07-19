use bevy::{platform::collections::HashMap, prelude::*};

use crate::{item::Item, quad::Direction};

use super::{RENDER_ID_COUNT, WATER_RENDER_ID, from_render_id, render_id_for_block};

#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockTextureLayer(u32);

impl BlockTextureLayer {
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockTextureAnimation {
    base_layer: BlockTextureLayer,
    frame_count: u32,
}

impl BlockTextureAnimation {
    pub const fn new(base_layer: BlockTextureLayer, frame_count: u32) -> Self {
        Self {
            base_layer,
            frame_count,
        }
    }

    pub const fn base_layer(self) -> BlockTextureLayer {
        self.base_layer
    }

    pub const fn frame_count(self) -> u32 {
        self.frame_count
    }
}

#[derive(Resource)]
pub struct BlockTextureMap(pub HashMap<String, BlockTextureAnimation>);

pub fn render_id_to_texture_path(rid: u16, side: Direction) -> &'static str {
    use Direction::*;
    if rid == WATER_RENDER_ID {
        return match side {
            Up | Down => "textures/block/water_still.png",
            _ => "textures/block/water_flow.png",
        };
    }
    from_render_id(rid)
        .expect("invalid render_id for texture")
        .texture_path(side)
}

impl BlockTextureMap {
    pub fn block_to_texture_animation(
        &self,
        block: Item,
        side: Direction,
    ) -> BlockTextureAnimation {
        self.render_id_to_texture_animation(render_id_for_block(block), side)
    }

    pub fn render_id_to_texture_animation(
        &self,
        rid: u16,
        side: Direction,
    ) -> BlockTextureAnimation {
        let path = render_id_to_texture_path(rid, side);
        self.0
            .get(path)
            .copied()
            .unwrap_or_else(|| panic!("missing texture for rid={rid} {side:?}: {path}"))
    }

    pub fn block_to_texture_layer(&self, block: Item, side: Direction) -> BlockTextureLayer {
        self.block_to_texture_animation(block, side).base_layer()
    }

    pub fn render_id_to_texture_layer(&self, rid: u16, side: Direction) -> BlockTextureLayer {
        self.render_id_to_texture_animation(rid, side).base_layer()
    }
}

/// GPU-ready face lookup shared by terrain and dropped block-items.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockVisualTable {
    pub texture_layers: Vec<u32>,
    pub tint_colors: Vec<[f32; 4]>,
    pub emission_factors: Vec<f32>,
}

impl BlockVisualTable {
    pub fn build(texture_map: &BlockTextureMap) -> Self {
        let entry_count = RENDER_ID_COUNT * Direction::COUNT;
        let mut texture_layers =
            vec![pack_texture_layer(BlockTextureLayer::default(), 1); entry_count];
        let mut tint_colors = vec![[0.0; 4]; entry_count];
        let mut emission_factors = vec![0.0; entry_count];

        for item in Item::BLOCKS {
            let render_id = render_id_for_block(item);
            for side in Direction::ALL {
                let index = render_id as usize * Direction::COUNT + side.index();
                let animation = texture_map.block_to_texture_animation(item, side);
                let tint = item.tint(side);
                texture_layers[index] =
                    pack_texture_layer(animation.base_layer(), animation.frame_count());
                tint_colors[index] = [tint.x, tint.y, tint.z, tint.w];
                emission_factors[index] = f32::from(item.light_emission()) / 15.0;
            }
        }

        for side in Direction::ALL {
            let index = WATER_RENDER_ID as usize * Direction::COUNT + side.index();
            let animation = texture_map.render_id_to_texture_animation(WATER_RENDER_ID, side);
            let tint = render_id_to_colour(WATER_RENDER_ID, side);
            texture_layers[index] =
                pack_texture_layer(animation.base_layer(), animation.frame_count());
            tint_colors[index] = [tint.x, tint.y, tint.z, tint.w];
        }

        Self {
            texture_layers,
            tint_colors,
            emission_factors,
        }
    }
}

pub fn pack_texture_layer(layer: BlockTextureLayer, frame_count: u32) -> u32 {
    layer.index() | (frame_count.min(255) << 24)
}

pub fn render_id_to_colour(rid: u16, side: Direction) -> Vec4 {
    if rid == WATER_RENDER_ID {
        return Srgba::hex("55B8FF").unwrap().with_alpha(0.62).to_vec4();
    }
    from_render_id(rid)
        .expect("invalid render_id for colour")
        .tint(side)
}
