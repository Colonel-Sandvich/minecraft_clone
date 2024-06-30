use bevy::{prelude::*, utils::HashMap};
use rand::Rng;
use strum::{EnumCount, EnumIter, FromRepr};

use crate::quad::Direction;

pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<BlockUpdateEvent>();
    }
}

#[derive(Clone, Copy, Debug, Reflect, PartialEq, FromRepr, EnumCount, EnumIter)]
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

    pub fn is_visible(&self) -> bool {
        self.visibility() != Visibility::Empty
    }

    pub fn is_solid(&self) -> bool {
        use BlockType::*;
        match self {
            Air => false,
            _ => true,
        }
    }

    pub fn random_not_air() -> Self {
        let mut rng = rand::thread_rng();

        BlockType::from_repr(rng.gen_range(1..BlockType::COUNT)).unwrap()
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
        *self
            .0
            .get(block_and_side_to_texture_path(block, side))
            .unwrap()
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

#[derive(Debug, Component, Deref, PartialEq)]
pub struct LocalBlockPos(pub UVec3);

impl LocalBlockPos {
    pub fn new(x: usize, y: usize, z: usize) -> Self {
        Self(UVec3::new(x as u32, y as u32, z as u32))
    }
}

impl Into<LocalBlockPos> for UVec3 {
    fn into(self) -> LocalBlockPos {
        LocalBlockPos(self)
    }
}

#[derive(Debug)]
pub struct Block {
    pub kind: BlockType,
    pub pos: LocalBlockPos,
}

impl Block {
    pub fn new(block: BlockType, pos: LocalBlockPos) -> Self {
        Self { kind: block, pos }
    }
}

pub enum BlockUpdateKind {
    Break,
    Place(BlockType),
    // Replace(BlockType, BlockType), // (old, new) ?
}

#[derive(Event)]
pub struct BlockUpdateEvent {
    pub chunk: Entity,
    pub pos: LocalBlockPos,
    pub kind: BlockUpdateKind,
}
