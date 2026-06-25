use bevy::{platform::collections::HashMap, prelude::*};
use strum::{Display, EnumCount, EnumIter, EnumString, FromRepr};

use crate::{quad::Direction, world::chunk::CHUNK_ISIZE};

pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BlockUpdateMessage>();
    }
}

#[derive(
    Default,
    Clone,
    Copy,
    Debug,
    Display,
    Reflect,
    PartialEq,
    Eq,
    Hash,
    EnumIter,
    EnumCount,
    EnumString,
    FromRepr,
)]
#[repr(u16)]
#[strum(serialize_all = "snake_case")]
pub enum BlockType {
    #[default]
    Grass = 0,
    Dirt,
    Stone,
    Sand,
    Glass,
    OakLog,
    OakLeaves,
    Glowstone,
    Ice,
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockStateId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HotBlockStateMeta {
    pub render_id: u16,
    pub mesh_flags: u8,
    pub light_opacity: u8,
    pub light_emission: u8,
    pub fluid_level: u8,
}

impl HotBlockStateMeta {
    pub const AIR: Self = Self {
        render_id: 0,
        mesh_flags: 0,
        light_opacity: 0,
        light_emission: 0,
        fluid_level: 0,
    };

    pub const fn for_block(block: BlockType) -> Self {
        Self {
            render_id: render_id_for_block(block),
            mesh_flags: block.mesh_flags(),
            light_opacity: block.light_opacity(),
            light_emission: block.light_emission(),
            fluid_level: 0,
        }
    }

    pub const fn water(level: u8) -> Self {
        Self {
            render_id: WATER_RENDER_ID,
            mesh_flags: BLOCK_FLAG_RENDERED | BLOCK_FLAG_TRANSLUCENT,
            light_opacity: 0,
            light_emission: 0,
            fluid_level: level,
        }
    }
}

pub const fn render_id_for_block(block: BlockType) -> u16 {
    block as u16 + 1
}

pub fn from_render_id(rid: u16) -> Option<BlockType> {
    BlockType::from_repr(rid.checked_sub(1)?)
}

pub const WATER_RENDER_ID: u16 = BlockType::COUNT as u16 + 1;
pub const RENDER_ID_COUNT: usize = BlockType::COUNT + 2;

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

impl BlockType {
    #[inline(always)]
    pub const fn is_rendered(self) -> bool {
        true
    }

    #[inline(always)]
    pub const fn is_full_cube(self) -> bool {
        matches!(
            self,
            Self::Grass | Self::Dirt | Self::Stone | Self::Sand | Self::OakLog | Self::Glowstone
        )
    }

    #[inline(always)]
    pub const fn light_emission(self) -> u8 {
        match self {
            Self::Glowstone => 15,
            _ => 0,
        }
    }

    #[inline(always)]
    pub const fn light_opacity(self) -> u8 {
        match self {
            Self::Ice => 0,
            Self::Glass => 0,
            Self::OakLeaves => 1,
            _ => 15,
        }
    }

    #[inline(always)]
    pub const fn is_transparent_to_sky_light(self) -> bool {
        match self {
            Self::Glass | Self::OakLeaves | Self::Ice => true,
            _ => false,
        }
    }

    #[inline(always)]
    pub const fn is_opaque_to_light(self) -> bool {
        self.light_opacity() >= 15
    }

    #[inline(always)]
    pub const fn emits_internal_faces(self) -> bool {
        matches!(self, Self::OakLeaves)
    }

    #[inline(always)]
    pub const fn material_layer_index(self) -> usize {
        match self {
            Self::Glass | Self::OakLeaves => 1,
            Self::Ice => 2,
            _ => 0,
        }
    }

    pub const fn render_profile(self) -> Option<BlockRenderProfile> {
        use BlockType::*;
        match self {
            Glass => Some(BlockRenderProfile {
                layer: BlockRenderLayer::Cutout,
                occlusion: FaceOcclusion::None,
            }),
            OakLeaves => Some(BlockRenderProfile {
                layer: BlockRenderLayer::Cutout,
                occlusion: FaceOcclusion::None,
            }),
            Ice => Some(BlockRenderProfile {
                layer: BlockRenderLayer::Translucent,
                occlusion: FaceOcclusion::None,
            }),
            _ => Some(BlockRenderProfile {
                layer: BlockRenderLayer::Opaque,
                occlusion: FaceOcclusion::FullCube,
            }),
        }
    }

    pub const fn is_solid(self) -> bool {
        true
    }

    pub const fn is_placeable(self) -> bool {
        true
    }

    pub const fn storage_id(self) -> u16 {
        self as u16
    }

    pub fn from_storage_id(id: u16) -> Option<Self> {
        Self::from_repr(id)
    }

    /// Global string identifier — stable across enum reorderings.
    /// Example: `"grass"`, `"oak_log"`.
    pub fn name(self) -> String {
        self.to_string()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        use std::str::FromStr;
        Self::from_str(name).ok()
    }

    pub const fn mesh_flags(self) -> u8 {
        let mut f = BLOCK_FLAG_RENDERED;
        if self.is_full_cube() {
            f |= BLOCK_FLAG_FULL_CUBE;
        }
        if self.emits_internal_faces() {
            f |= BLOCK_FLAG_EMITS_INTERNAL_FACES;
        }
        f |= match self.material_layer_index() {
            1 => BLOCK_FLAG_CUTOUT,
            2 => BLOCK_FLAG_TRANSLUCENT,
            _ => 0,
        };
        f
    }
}

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
    block_and_side_to_texture_path(
        from_render_id(rid).expect("invalid render_id for texture"),
        side,
    )
}

pub fn block_and_side_to_texture_path(block: BlockType, side: Direction) -> &'static str {
    use BlockType::*;
    use Direction::*;
    match block {
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
        Glowstone => "textures/block/glowstone.png",
        Ice => "textures/block/ice.png",
    }
}

impl BlockTextureMap {
    pub fn block_to_texture_animation(
        &self,
        block: BlockType,
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

    pub fn block_to_texture_layer(&self, block: BlockType, side: Direction) -> BlockTextureLayer {
        self.block_to_texture_animation(block, side).base_layer()
    }

    pub fn render_id_to_texture_layer(&self, rid: u16, side: Direction) -> BlockTextureLayer {
        self.render_id_to_texture_animation(rid, side).base_layer()
    }
}

pub fn render_id_to_colour(rid: u16, side: Direction) -> Vec4 {
    if rid == WATER_RENDER_ID {
        return Srgba::hex("55B8FF").unwrap().with_alpha(0.62).to_vec4();
    }
    block_to_colour(
        from_render_id(rid).expect("invalid render_id for colour"),
        side,
    )
}

pub fn block_to_colour(block: BlockType, side: Direction) -> Vec4 {
    use BlockType::*;
    use Direction::*;

    let color = match block {
        Grass => match side {
            Up => Srgba::hex("5E9D34").unwrap(),
            _ => Srgba::WHITE,
        },
        OakLeaves => Srgba::hex("77AB2F").unwrap(),
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

        assert_eq!(BlockType::Grass.storage_id(), 0);
        assert_eq!(BlockType::Dirt.storage_id(), 1);
        assert_eq!(BlockType::Stone.storage_id(), 2);
        assert_eq!(BlockType::Sand.storage_id(), 3);
        assert_eq!(BlockType::Glass.storage_id(), 4);
        assert_eq!(BlockType::OakLog.storage_id(), 5);
        assert_eq!(BlockType::OakLeaves.storage_id(), 6);
        assert_eq!(BlockType::Glowstone.storage_id(), 7);
        assert_eq!(BlockType::Ice.storage_id(), 8);
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

    #[test]
    fn water_is_translucent_non_solid_and_non_occluding() {
        let profile = BlockRenderProfile {
            layer: BlockRenderLayer::Translucent,
            occlusion: FaceOcclusion::None,
        };
        assert_eq!(profile.material_layer(), BlockMaterialLayer::Translucent);
    }

    #[test]
    fn ice_is_translucent_solid_and_non_occluding() {
        let profile = BlockType::Ice.render_profile().unwrap();
        assert_eq!(
            profile,
            BlockRenderProfile {
                layer: BlockRenderLayer::Translucent,
                occlusion: FaceOcclusion::None,
            }
        );
        assert_eq!(profile.material_layer(), BlockMaterialLayer::Translucent);
        assert!(BlockType::Ice.is_solid());
        assert!(BlockType::Ice.is_placeable());
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
}

#[derive(Message)]
pub struct BlockUpdateMessage {
    pub chunk: Entity,
    pub pos: BlockPos,
    pub kind: BlockUpdateKind,
}
