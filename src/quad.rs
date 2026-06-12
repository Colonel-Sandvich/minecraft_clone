use bevy::math::{IVec3, Rect, UVec3, Vec2, Vec3, Vec4, vec2};
use strum::EnumIter;

#[derive(Copy, Clone, Debug)]
pub struct Quad {
    pub voxel: UVec3,
    pub color: Vec4,
    pub uv: Rect,
    pub ao: [u8; 4],
    // pub width: u32,
    // pub height: u32,
}

#[derive(Debug, Default)]
pub struct QuadGroups {
    pub groups: [Vec<Quad>; 6],
}

pub fn get_indices(start: u32) -> [u32; 6] {
    [start, start + 2, start + 1, start + 1, start + 2, start + 3]
}

pub fn get_positions(quad: &Quad, side: &Direction, voxel_size: f32) -> [Vec3; 4] {
    get_vertex_offsets(*side).map(|pos| (pos.as_vec3() + quad.voxel.as_vec3()) * voxel_size)
}

pub fn get_vertex_offsets(side: Direction) -> [IVec3; 4] {
    use Direction::*;
    match side {
        Left => [
            IVec3::new(0, 0, 1),
            IVec3::new(0, 0, 0),
            IVec3::new(0, 1, 1),
            IVec3::new(0, 1, 0),
        ],
        Right => [
            IVec3::new(1, 0, 0),
            IVec3::new(1, 0, 1),
            IVec3::new(1, 1, 0),
            IVec3::new(1, 1, 1),
        ],
        Down => [
            IVec3::new(0, 0, 1),
            IVec3::new(1, 0, 1),
            IVec3::new(0, 0, 0),
            IVec3::new(1, 0, 0),
        ],
        Up => [
            IVec3::new(0, 1, 1),
            IVec3::new(0, 1, 0),
            IVec3::new(1, 1, 1),
            IVec3::new(1, 1, 0),
        ],
        Forward => [
            IVec3::new(0, 0, 0),
            IVec3::new(1, 0, 0),
            IVec3::new(0, 1, 0),
            IVec3::new(1, 1, 0),
        ],
        Backward => [
            IVec3::new(1, 0, 1),
            IVec3::new(0, 0, 1),
            IVec3::new(1, 1, 1),
            IVec3::new(0, 1, 1),
        ],
    }
}

pub type Normals = [Vec3; 4];

pub fn get_normals(side: Vec3) -> Normals {
    assert_eq!(side.length(), 1.0);
    [side; 4]
}

pub type UVs = [Vec2; 4];

pub fn urect_to_uvs(rect: &Rect) -> UVs {
    let Rect { min, max } = *rect;
    let Vec2 { x: x0, y: y0 } = min;
    let Vec2 { x: x1, y: y1 } = max;
    [vec2(x0, y1), vec2(x1, y1), vec2(x0, y0), vec2(x1, y0)]
}

#[derive(Debug, Clone, Copy, PartialEq, EnumIter)]
pub enum Direction {
    Left,
    Right,
    Down,
    Up,
    Forward,
    Backward, // Probably wrong
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
