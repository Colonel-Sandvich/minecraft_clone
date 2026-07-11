use bevy::math::Vec3;
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

    pub const fn index(self) -> usize {
        self as usize
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
