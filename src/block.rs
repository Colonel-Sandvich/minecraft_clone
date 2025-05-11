use bevy::{platform::collections::HashMap, prelude::*};
use rand::Rng;
use strum::{EnumCount, EnumIter, FromRepr};

use crate::{chunk::CHUNK_ISIZE, quad::Direction};

pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<BlockUpdateEvent>();
    }
}

#[derive(Default, Clone, Copy, Debug, Reflect, PartialEq, FromRepr, EnumCount, EnumIter)]
pub enum BlockType {
    #[default]
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
        let mut rng = rand::rng();

        BlockType::from_repr(rng.random_range(1..BlockType::COUNT)).unwrap()
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

    let color = match block {
        Grass => match side {
            Up => Srgba::hex("5E9D34").unwrap(),
            _ => Srgba::WHITE,
        },
        _ => Srgba::WHITE,
    };

    color.to_vec4()
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

#[derive(Event)]
pub struct BlockUpdateEvent {
    pub chunk: Entity,
    pub pos: BlockPos,
    pub kind: BlockUpdateKind,
}
