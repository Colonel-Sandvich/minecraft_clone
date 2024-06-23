use bevy::{prelude::*, utils::HashMap};
use strum::{EnumCount, EnumIter, FromRepr};

use crate::quad::Direction;

pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn build(&self, app: &mut App) {}
}

#[derive(Clone, Copy, Debug, PartialEq, FromRepr, EnumCount, EnumIter)]
pub enum BlockType {
    Air = 0,
    Grass,
    Dirt,
    Stone,
    Sand,
    Glass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    Empty,
    Translucent,
    Opaque,
}

impl BlockType {
    pub fn visibility(&self) -> Visibility {
        use BlockType::*;
        match self {
            Air => Visibility::Empty,
            Glass => Visibility::Translucent,
            _ => Visibility::Opaque,
        }
    }

    pub fn visible(&self) -> bool {
        self.visibility() != Visibility::Empty
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
    }
}

impl BlockTextureMap {
    pub fn block_to_mesh(&self, block: BlockType, side: Direction) -> Rect {
        self.0
            .get(block_and_side_to_texture_path(block, side))
            .unwrap()
            .clone()
    }
}

pub fn block_to_colour(block: BlockType, side: Direction) -> Vec4 {
    use BlockType::*;
    use Direction::*;

    match block {
        Grass => match side {
            Up => Color::hex("5E9D34").unwrap(),
            _ => Color::WHITE,
        },
        _ => Color::WHITE,
    }
    .rgba_to_vec4()
}
