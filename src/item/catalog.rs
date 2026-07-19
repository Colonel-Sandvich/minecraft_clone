use bevy::prelude::*;
use strum::{Display, EnumIter, EnumString};

use crate::{
    block::{
        BLOCK_FLAG_CUTOUT, BLOCK_FLAG_EMITS_INTERNAL_FACES, BLOCK_FLAG_FULL_CUBE,
        BLOCK_FLAG_RENDERED, BLOCK_FLAG_TRANSLUCENT, BlockMaterialLayer, BlockRenderLayer,
        BlockRenderProfile, FaceOcclusion,
    },
    quad::Direction,
};

/// Stable gameplay identity for anything a player can hold.
///
/// Only [`Item::BLOCKS`] may be stored in a chunk. Keeping that subset explicit
/// lets tools, food, and other non-block items be added without changing block
/// state IDs in saved worlds.
#[derive(
    Default, Clone, Copy, Debug, Display, Reflect, PartialEq, Eq, Hash, EnumIter, EnumString,
)]
#[strum(serialize_all = "snake_case")]
pub enum Item {
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

impl Item {
    /// Blocks in their stable chunk-storage order.
    ///
    /// Non-block items may be added anywhere in [`Item`] without affecting
    /// these IDs. Append new blocks here; never reorder existing entries.
    pub const BLOCKS: [Self; 9] = [
        Self::Grass,
        Self::Dirt,
        Self::Stone,
        Self::Sand,
        Self::Glass,
        Self::OakLog,
        Self::OakLeaves,
        Self::Glowstone,
        Self::Ice,
    ];
    pub const BLOCK_COUNT: usize = Self::BLOCKS.len();

    pub const fn is_block(self) -> bool {
        matches!(
            self,
            Self::Grass
                | Self::Dirt
                | Self::Stone
                | Self::Sand
                | Self::Glass
                | Self::OakLog
                | Self::OakLeaves
                | Self::Glowstone
                | Self::Ice
        )
    }

    pub const fn block_storage_id(self) -> Option<u16> {
        match self {
            Self::Grass => Some(0),
            Self::Dirt => Some(1),
            Self::Stone => Some(2),
            Self::Sand => Some(3),
            Self::Glass => Some(4),
            Self::OakLog => Some(5),
            Self::OakLeaves => Some(6),
            Self::Glowstone => Some(7),
            Self::Ice => Some(8),
        }
    }

    pub const fn from_block_storage_id(id: u16) -> Option<Self> {
        if (id as usize) < Self::BLOCK_COUNT {
            Some(Self::BLOCKS[id as usize])
        } else {
            None
        }
    }

    pub const fn is_full_cube(self) -> bool {
        matches!(
            self,
            Self::Grass | Self::Dirt | Self::Stone | Self::Sand | Self::OakLog | Self::Glowstone
        )
    }

    pub const fn light_emission(self) -> u8 {
        match self {
            Self::Glowstone => 15,
            _ => 0,
        }
    }

    pub const fn light_opacity(self) -> u8 {
        match self {
            Self::Ice | Self::Glass => 0,
            Self::OakLeaves => 1,
            _ => 15,
        }
    }

    pub const fn is_transparent_to_sky_light(self) -> bool {
        matches!(self, Self::Glass | Self::OakLeaves | Self::Ice)
    }

    pub const fn is_opaque_to_light(self) -> bool {
        self.light_opacity() >= 15
    }

    pub const fn emits_internal_faces(self) -> bool {
        matches!(self, Self::OakLeaves)
    }

    pub const fn render_profile(self) -> Option<BlockRenderProfile> {
        if !self.is_block() {
            return None;
        }
        Some(match self {
            Self::Glass | Self::OakLeaves => BlockRenderProfile {
                layer: BlockRenderLayer::Cutout,
                occlusion: FaceOcclusion::None,
            },
            Self::Ice => BlockRenderProfile {
                layer: BlockRenderLayer::Translucent,
                occlusion: FaceOcclusion::None,
            },
            _ => BlockRenderProfile {
                layer: BlockRenderLayer::Opaque,
                occlusion: FaceOcclusion::FullCube,
            },
        })
    }

    pub const fn material_layer(self) -> Option<BlockMaterialLayer> {
        match self.render_profile() {
            Some(profile) => Some(profile.material_layer()),
            None => None,
        }
    }

    pub const fn is_solid(self) -> bool {
        self.is_block()
    }

    pub const fn is_placeable(self) -> bool {
        self.is_block()
    }

    pub fn name(self) -> String {
        self.to_string()
    }

    pub fn from_name(name: &str) -> Option<Self> {
        use std::str::FromStr;
        Self::from_str(name).ok()
    }

    pub const fn mesh_flags(self) -> u8 {
        if !self.is_block() {
            return 0;
        }
        let mut flags = BLOCK_FLAG_RENDERED;
        if self.is_full_cube() {
            flags |= BLOCK_FLAG_FULL_CUBE;
        }
        if self.emits_internal_faces() {
            flags |= BLOCK_FLAG_EMITS_INTERNAL_FACES;
        }
        flags |= match self.material_layer() {
            Some(BlockMaterialLayer::Cutout) => BLOCK_FLAG_CUTOUT,
            Some(BlockMaterialLayer::Translucent) => BLOCK_FLAG_TRANSLUCENT,
            Some(BlockMaterialLayer::Opaque) | None => 0,
        };
        flags
    }

    /// Texture used by terrain, dropped block-items, and generated UI icons.
    pub const fn texture_path(self, side: Direction) -> &'static str {
        use Direction::*;
        match self {
            Self::Dirt => "textures/block/dirt.png",
            Self::Grass => match side {
                Up => "textures/block/grass_block_top.png",
                Down => "textures/block/dirt.png",
                _ => "textures/block/grass_block_side.png",
            },
            Self::Sand => "textures/block/sand.png",
            Self::Stone => "textures/block/stone.png",
            Self::Glass => "textures/block/glass.png",
            Self::OakLog => match side {
                Up | Down => "textures/block/oak_log_top.png",
                _ => "textures/block/oak_log.png",
            },
            Self::OakLeaves => "textures/block/oak_leaves.png",
            Self::Glowstone => "textures/block/glowstone.png",
            Self::Ice => "textures/block/ice.png",
        }
    }

    /// Per-face tint shared by terrain, dropped block-items, and UI icons.
    pub fn tint(self, side: Direction) -> Vec4 {
        let color = match self {
            Self::Grass if side == Direction::Up => Srgba::hex("5E9D34").unwrap(),
            Self::OakLeaves => Srgba::hex("77AB2F").unwrap(),
            _ => Srgba::WHITE,
        };
        color.to_vec4()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_storage_ids_are_stable_and_roundtrip() {
        for item in Item::BLOCKS {
            assert_eq!(
                Item::from_block_storage_id(item.block_storage_id().unwrap()),
                Some(item)
            );
        }

        assert_eq!(Item::Grass.block_storage_id(), Some(0));
        assert_eq!(Item::Dirt.block_storage_id(), Some(1));
        assert_eq!(Item::Stone.block_storage_id(), Some(2));
        assert_eq!(Item::Sand.block_storage_id(), Some(3));
        assert_eq!(Item::Glass.block_storage_id(), Some(4));
        assert_eq!(Item::OakLog.block_storage_id(), Some(5));
        assert_eq!(Item::OakLeaves.block_storage_id(), Some(6));
        assert_eq!(Item::Glowstone.block_storage_id(), Some(7));
        assert_eq!(Item::Ice.block_storage_id(), Some(8));
    }

    #[test]
    fn grass_and_logs_define_face_specific_textures() {
        assert_ne!(
            Item::Grass.texture_path(Direction::Up),
            Item::Grass.texture_path(Direction::Down)
        );
        assert_ne!(
            Item::OakLog.texture_path(Direction::Up),
            Item::OakLog.texture_path(Direction::Right)
        );
        assert_eq!(
            Item::OakLog.texture_path(Direction::Up),
            Item::OakLog.texture_path(Direction::Down)
        );
    }

    #[test]
    fn leaves_are_cutout_and_non_occluding() {
        let profile = Item::OakLeaves.render_profile().unwrap();
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
    fn ice_is_translucent_solid_and_non_occluding() {
        let profile = Item::Ice.render_profile().unwrap();
        assert_eq!(
            profile,
            BlockRenderProfile {
                layer: BlockRenderLayer::Translucent,
                occlusion: FaceOcclusion::None,
            }
        );
        assert_eq!(profile.material_layer(), BlockMaterialLayer::Translucent);
        assert!(Item::Ice.is_solid());
        assert!(Item::Ice.is_placeable());
    }
}
