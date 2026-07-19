pub const BLOCK_FLAG_RENDERED: u8 = 1 << 0;
pub const BLOCK_FLAG_FULL_CUBE: u8 = 1 << 1;
pub const BLOCK_FLAG_EMITS_INTERNAL_FACES: u8 = 1 << 2;
pub const BLOCK_FLAG_CUTOUT: u8 = 1 << 3;
pub const BLOCK_FLAG_TRANSLUCENT: u8 = 1 << 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockRenderLayer {
    Opaque,
    Cutout,
    Translucent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceOcclusion {
    None,
    FullCube,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRenderProfile {
    pub layer: BlockRenderLayer,
    pub occlusion: FaceOcclusion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockMaterialLayer {
    Opaque,
    Cutout,
    Translucent,
}

impl BlockMaterialLayer {
    pub const COUNT: usize = 3;
    pub const ALL: [Self; Self::COUNT] = [Self::Opaque, Self::Cutout, Self::Translucent];

    pub const fn index(self) -> usize {
        match self {
            Self::Opaque => 0,
            Self::Cutout => 1,
            Self::Translucent => 2,
        }
    }
}

impl BlockRenderProfile {
    pub const fn material_layer(self) -> BlockMaterialLayer {
        match self.layer {
            BlockRenderLayer::Opaque => BlockMaterialLayer::Opaque,
            BlockRenderLayer::Cutout => BlockMaterialLayer::Cutout,
            BlockRenderLayer::Translucent => BlockMaterialLayer::Translucent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_layers_map_to_matching_material_layers() {
        for (layer, expected) in [
            (BlockRenderLayer::Opaque, BlockMaterialLayer::Opaque),
            (BlockRenderLayer::Cutout, BlockMaterialLayer::Cutout),
            (
                BlockRenderLayer::Translucent,
                BlockMaterialLayer::Translucent,
            ),
        ] {
            let profile = BlockRenderProfile {
                layer,
                occlusion: FaceOcclusion::None,
            };
            assert_eq!(profile.material_layer(), expected);
        }
    }
}
