use bevy::{platform::collections::HashMap, prelude::*};
use strum::EnumIter;

use crate::{quad::Direction, world::chunk::CHUNK_ISIZE};

pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BlockUpdateMessage>();
    }
}

#[derive(Default, Clone, Copy, Debug, Reflect, PartialEq, Eq, EnumIter)]
pub enum BlockType {
    #[default]
    Air = 0,
    Grass,
    Dirt,
    Stone,
    Sand,
    Glass,
    OakLog,
    OakLeaves,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockRenderLayer {
    Opaque,
    Cutout,
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
}

impl BlockMaterialLayer {
    pub const COUNT: usize = 2;
    pub const ALL: [Self; Self::COUNT] = [Self::Opaque, Self::Cutout];

    pub const fn index(self) -> usize {
        match self {
            Self::Opaque => 0,
            Self::Cutout => 1,
        }
    }
}

impl BlockRenderProfile {
    pub const fn material_layer(self) -> BlockMaterialLayer {
        match self.layer {
            BlockRenderLayer::Opaque => BlockMaterialLayer::Opaque,
            BlockRenderLayer::Cutout => BlockMaterialLayer::Cutout,
        }
    }
}

impl BlockType {
    #[inline(always)]
    pub const fn is_rendered(self) -> bool {
        !matches!(self, Self::Air)
    }

    #[inline(always)]
    pub const fn is_full_cube(self) -> bool {
        matches!(
            self,
            Self::Grass | Self::Dirt | Self::Stone | Self::Sand | Self::OakLog
        )
    }

    #[inline(always)]
    pub const fn emits_internal_faces(self) -> bool {
        matches!(self, Self::OakLeaves)
    }

    #[inline(always)]
    pub const fn material_layer_index(self) -> usize {
        match self {
            Self::Glass | Self::OakLeaves => 1,
            _ => 0,
        }
    }

    pub const fn render_profile(self) -> Option<BlockRenderProfile> {
        use BlockType::*;
        match self {
            Air => None,
            Glass => Some(BlockRenderProfile {
                layer: BlockRenderLayer::Cutout,
                occlusion: FaceOcclusion::None,
            }),
            OakLeaves => Some(BlockRenderProfile {
                layer: BlockRenderLayer::Cutout,
                occlusion: FaceOcclusion::None,
            }),
            _ => Some(BlockRenderProfile {
                layer: BlockRenderLayer::Opaque,
                occlusion: FaceOcclusion::FullCube,
            }),
        }
    }

    pub fn is_solid(&self) -> bool {
        use BlockType::*;
        match self {
            Air => false,
            _ => true,
        }
    }

    pub const fn storage_id(self) -> u16 {
        use BlockType::*;
        match self {
            Air => 0,
            Grass => 1,
            Dirt => 2,
            Stone => 3,
            Sand => 4,
            Glass => 5,
            OakLog => 6,
            OakLeaves => 7,
        }
    }

    pub const fn from_storage_id(id: u16) -> Option<Self> {
        use BlockType::*;
        match id {
            0 => Some(Air),
            1 => Some(Grass),
            2 => Some(Dirt),
            3 => Some(Stone),
            4 => Some(Sand),
            5 => Some(Glass),
            6 => Some(OakLog),
            7 => Some(OakLeaves),
            _ => None,
        }
    }
}

#[derive(Resource)]
pub struct BlockTextureMap(pub HashMap<String, Rect>);

pub fn block_and_side_to_texture_path(block: BlockType, side: Direction) -> &'static str {
    use BlockType::*;
    use Direction::*;
    match block {
        Air => unreachable!(),
        Dirt => "textures/block/dirt.png",
        Grass => match side {
            Up => "textures/block/grass_block_top.png",
            Down => "textures/block/dirt.png",
            _ => "textures/block/grass_block_side.png",
        },
        Sand => "textures/block/sand.png",
        Stone => "textures/block/stone.png",
        Glass => "textures/block/glass.png",
        OakLog => match side {
            Up | Down => "textures/block/oak_log_top.png",
            _ => "textures/block/oak_log.png",
        },
        OakLeaves => "textures/block/oak_leaves.png",
    }
}

impl BlockTextureMap {
    pub fn block_to_mesh(&self, block: BlockType, side: Direction) -> Rect {
        let path = block_and_side_to_texture_path(block, side);
        self.0
            .get(path)
            .copied()
            .unwrap_or_else(|| panic!("missing texture for {block:?} {side:?}: {path}"))
    }
}

pub fn block_to_colour(block: BlockType, side: Direction) -> Vec4 {
    use BlockType::*;
    use Direction::*;

    let color = match block {
        Grass => match side {
            Up => Srgba::hex("5E9D34").unwrap(),
            _ => Srgba::WHITE,
        },
        _ => Srgba::WHITE,
    };

    color.to_vec4()
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn block_storage_ids_are_stable_and_roundtrip() {
        for block in BlockType::iter() {
            assert_eq!(BlockType::from_storage_id(block.storage_id()), Some(block));
        }

        assert_eq!(BlockType::Air.storage_id(), 0);
        assert_eq!(BlockType::Grass.storage_id(), 1);
        assert_eq!(BlockType::Dirt.storage_id(), 2);
        assert_eq!(BlockType::Stone.storage_id(), 3);
        assert_eq!(BlockType::Sand.storage_id(), 4);
        assert_eq!(BlockType::Glass.storage_id(), 5);
        assert_eq!(BlockType::OakLog.storage_id(), 6);
        assert_eq!(BlockType::OakLeaves.storage_id(), 7);
    }

    #[test]
    fn leaves_are_cutout_and_non_occluding() {
        let profile = BlockType::OakLeaves.render_profile().unwrap();
        assert_eq!(
            profile,
            BlockRenderProfile {
                layer: BlockRenderLayer::Cutout,
                occlusion: FaceOcclusion::None,
            }
        );
        assert_eq!(profile.material_layer(), BlockMaterialLayer::Cutout);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockPos {
    pub chunk: IVec3,
    pub block: UVec3,
}

impl BlockPos {
    pub fn from_global(pos: IVec3) -> Self {
        let chunk = (pos.as_vec3() / CHUNK_ISIZE as f32).floor().as_ivec3();
        let block = (pos - chunk * CHUNK_ISIZE).as_uvec3();

        Self { chunk, block }
    }

    pub fn to_global(&self) -> IVec3 {
        self.chunk * CHUNK_ISIZE + self.block.as_ivec3()
    }
}

pub enum BlockUpdateKind {
    Break,
    Place(BlockType),
    // Replace(BlockType, BlockType), // (old, new) ?
}

#[derive(Message)]
pub struct BlockUpdateMessage {
    pub chunk: Entity,
    pub pos: BlockPos,
    pub kind: BlockUpdateKind,
}
