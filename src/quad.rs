use bevy::math::{IVec3, Vec3};
use strum::EnumIter;

/// Canonical face order shared by meshing tables and the terrain shader.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter)]
pub enum Direction {
    Left = 0,
    Right = 1,
    Down = 2,
    Up = 3,
    Forward = 4,
    Backward = 5,
}

impl Direction {
    pub const COUNT: usize = 6;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Left,
        Self::Right,
        Self::Down,
        Self::Up,
        Self::Forward,
        Self::Backward,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Down => Self::Up,
            Self::Up => Self::Down,
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
        }
    }

    pub const fn offset(self) -> IVec3 {
        match self {
            Self::Left => IVec3::NEG_X,
            Self::Right => IVec3::X,
            Self::Down => IVec3::NEG_Y,
            Self::Up => IVec3::Y,
            Self::Forward => IVec3::NEG_Z,
            Self::Backward => IVec3::Z,
        }
    }
}

impl From<Direction> for IVec3 {
    fn from(value: Direction) -> Self {
        value.offset()
    }
}

impl From<Direction> for Vec3 {
    fn from(val: Direction) -> Self {
        use Direction::*;
        match val {
            Left => Vec3::NEG_X,
            Right => Vec3::X,
            Down => Vec3::NEG_Y,
            Up => Vec3::Y,
            Forward => Vec3::NEG_Z,
            Backward => Vec3::Z,
        }
    }
}
