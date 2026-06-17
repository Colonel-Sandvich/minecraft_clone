use bevy::math::Vec3;
use strum::EnumIter;

#[derive(Debug, Clone, Copy, PartialEq, EnumIter)]
pub enum Direction {
    Left,
    Right,
    Down,
    Up,
    Forward,
    Backward,
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
