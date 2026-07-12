use std::ops::{Add, AddAssign, Sub, SubAssign};

use bevy::math::{IVec3, UVec3, Vec3};

use crate::quad::Direction;

pub const CHUNK_SIZE: usize = 16;
pub const CHUNK_ISIZE: i32 = CHUNK_SIZE as i32;
pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

macro_rules! impl_ivec3_point_ops {
    ($point:ty) => {
        impl Add<IVec3> for $point {
            type Output = Self;

            fn add(self, rhs: IVec3) -> Self::Output {
                self.offset(rhs)
            }
        }

        impl AddAssign<IVec3> for $point {
            fn add_assign(&mut self, rhs: IVec3) {
                *self = self.offset(rhs);
            }
        }

        impl Sub<IVec3> for $point {
            type Output = Self;

            fn sub(self, rhs: IVec3) -> Self::Output {
                self.offset(-rhs)
            }
        }

        impl SubAssign<IVec3> for $point {
            fn sub_assign(&mut self, rhs: IVec3) {
                *self = self.offset(-rhs);
            }
        }

        impl Sub<$point> for $point {
            type Output = IVec3;

            fn sub(self, rhs: $point) -> Self::Output {
                self.as_ivec3() - rhs.as_ivec3()
            }
        }
    };
}

/// A position on the chunk grid.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkPos(IVec3);

impl ChunkPos {
    pub const ZERO: Self = Self(IVec3::ZERO);

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self(IVec3::new(x, y, z))
    }

    pub const fn from_ivec3(value: IVec3) -> Self {
        Self(value)
    }

    pub const fn as_ivec3(self) -> IVec3 {
        self.0
    }

    pub const fn x(self) -> i32 {
        self.0.x
    }

    pub const fn y(self) -> i32 {
        self.0.y
    }

    pub const fn z(self) -> i32 {
        self.0.z
    }

    pub const fn with_y(self, y: i32) -> Self {
        Self::new(self.x(), y, self.z())
    }

    pub const fn column(self) -> ChunkColumn {
        ChunkColumn::new(self.x(), self.z())
    }

    pub fn containing_translation(translation: Vec3) -> Self {
        WorldBlockPos::from_translation(translation).split().chunk
    }

    pub fn origin(self) -> WorldBlockPos {
        WorldBlockPos(self.0 * CHUNK_ISIZE)
    }

    pub fn origin_translation(self) -> Vec3 {
        self.origin().as_ivec3().as_vec3()
    }

    pub fn offset(self, offset: IVec3) -> Self {
        Self(self.0 + offset)
    }

    pub const fn block(self, local: LocalBlockPos) -> ChunkBlockPos {
        ChunkBlockPos::new(self, local)
    }

    pub fn local_of(self, world: WorldBlockPos) -> Option<LocalBlockPos> {
        let address = world.split();
        if address.chunk == self {
            Some(address.local)
        } else {
            None
        }
    }
}

impl From<IVec3> for ChunkPos {
    fn from(value: IVec3) -> Self {
        Self::from_ivec3(value)
    }
}

impl From<ChunkPos> for IVec3 {
    fn from(value: ChunkPos) -> Self {
        value.as_ivec3()
    }
}

impl_ivec3_point_ops!(ChunkPos);

/// The XZ address of a vertical chunk column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkColumn {
    x: i32,
    z: i32,
}

impl ChunkColumn {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    pub const fn from_chunk(chunk: ChunkPos) -> Self {
        chunk.column()
    }

    pub const fn x(self) -> i32 {
        self.x
    }

    pub const fn z(self) -> i32 {
        self.z
    }

    pub const fn chunk(self, y: i32) -> ChunkPos {
        ChunkPos::new(self.x, y, self.z)
    }
}

impl From<ChunkPos> for ChunkColumn {
    fn from(chunk: ChunkPos) -> Self {
        Self::from_chunk(chunk)
    }
}

/// An integer block position in world coordinates.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorldBlockPos(IVec3);

impl WorldBlockPos {
    pub const ZERO: Self = Self(IVec3::ZERO);

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self(IVec3::new(x, y, z))
    }

    pub const fn from_ivec3(value: IVec3) -> Self {
        Self(value)
    }

    pub fn from_translation(translation: Vec3) -> Self {
        Self(translation.floor().as_ivec3())
    }

    pub const fn as_ivec3(self) -> IVec3 {
        self.0
    }

    pub const fn split(self) -> ChunkBlockPos {
        let chunk = ChunkPos::new(
            self.0.x.div_euclid(CHUNK_ISIZE),
            self.0.y.div_euclid(CHUNK_ISIZE),
            self.0.z.div_euclid(CHUNK_ISIZE),
        );
        let local = LocalBlockPos::new(
            self.0.x.rem_euclid(CHUNK_ISIZE) as u32,
            self.0.y.rem_euclid(CHUNK_ISIZE) as u32,
            self.0.z.rem_euclid(CHUNK_ISIZE) as u32,
        );
        ChunkBlockPos::new(chunk, local)
    }

    pub const fn chunk(self) -> ChunkPos {
        self.split().chunk
    }

    pub const fn local(self) -> LocalBlockPos {
        self.split().local
    }

    pub fn offset(self, offset: IVec3) -> Self {
        Self(self.0 + offset)
    }
}

impl From<IVec3> for WorldBlockPos {
    fn from(value: IVec3) -> Self {
        Self::from_ivec3(value)
    }
}

impl From<WorldBlockPos> for IVec3 {
    fn from(value: WorldBlockPos) -> Self {
        value.as_ivec3()
    }
}

impl_ivec3_point_ops!(WorldBlockPos);

/// A validated block position within one chunk.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalBlockPos(UVec3);

impl LocalBlockPos {
    pub const ZERO: Self = Self(UVec3::ZERO);
    pub const MAX: Self = Self(UVec3::splat(CHUNK_SIZE as u32 - 1));

    pub const fn new(x: u32, y: u32, z: u32) -> Self {
        assert!(x < CHUNK_SIZE as u32);
        assert!(y < CHUNK_SIZE as u32);
        assert!(z < CHUNK_SIZE as u32);
        Self(UVec3::new(x, y, z))
    }

    pub const fn try_from_uvec3(value: UVec3) -> Option<Self> {
        if value.x < CHUNK_SIZE as u32 && value.y < CHUNK_SIZE as u32 && value.z < CHUNK_SIZE as u32
        {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn as_uvec3(self) -> UVec3 {
        self.0
    }

    pub const fn x(self) -> usize {
        self.0.x as usize
    }

    pub const fn y(self) -> usize {
        self.0.y as usize
    }

    pub const fn z(self) -> usize {
        self.0.z as usize
    }

    pub const fn index(self) -> ChunkIndex {
        ChunkIndex::from_local(self)
    }

    pub const fn is_boundary(self) -> bool {
        self.0.x == 0
            || self.0.x == CHUNK_SIZE as u32 - 1
            || self.0.y == 0
            || self.0.y == CHUNK_SIZE as u32 - 1
            || self.0.z == 0
            || self.0.z == CHUNK_SIZE as u32 - 1
    }
}

impl TryFrom<UVec3> for LocalBlockPos {
    type Error = InvalidLocalBlockPos;

    fn try_from(value: UVec3) -> Result<Self, Self::Error> {
        Self::try_from_uvec3(value).ok_or(InvalidLocalBlockPos(value))
    }
}

impl From<LocalBlockPos> for UVec3 {
    fn from(value: LocalBlockPos) -> Self {
        value.as_uvec3()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidLocalBlockPos(pub UVec3);

impl std::fmt::Display for InvalidLocalBlockPos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "local block position is outside a {CHUNK_SIZE}^3 chunk: {}",
            self.0
        )
    }
}

impl std::error::Error for InvalidLocalBlockPos {}

/// A block address split into its chunk and validated local coordinates.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkBlockPos {
    chunk: ChunkPos,
    local: LocalBlockPos,
}

impl ChunkBlockPos {
    pub const fn new(chunk: ChunkPos, local: LocalBlockPos) -> Self {
        Self { chunk, local }
    }

    pub const fn from_world(world: WorldBlockPos) -> Self {
        world.split()
    }

    pub const fn chunk(self) -> ChunkPos {
        self.chunk
    }

    pub const fn local(self) -> LocalBlockPos {
        self.local
    }

    pub fn world(self) -> WorldBlockPos {
        WorldBlockPos(self.chunk.as_ivec3() * CHUNK_ISIZE + self.local.as_uvec3().as_ivec3())
    }

    pub fn offset(self, offset: IVec3) -> Self {
        self.world().offset(offset).split()
    }

    /// Steps to one face-adjacent block without normalizing through world coordinates.
    pub fn neighbor(self, direction: Direction) -> Self {
        let mut local = self.local.as_uvec3();
        let chunk = match direction {
            Direction::Left if local.x == 0 => {
                local.x = CHUNK_SIZE as u32 - 1;
                self.chunk.offset(IVec3::NEG_X)
            }
            Direction::Left => {
                local.x -= 1;
                self.chunk
            }
            Direction::Right if local.x == CHUNK_SIZE as u32 - 1 => {
                local.x = 0;
                self.chunk.offset(IVec3::X)
            }
            Direction::Right => {
                local.x += 1;
                self.chunk
            }
            Direction::Down if local.y == 0 => {
                local.y = CHUNK_SIZE as u32 - 1;
                self.chunk.offset(IVec3::NEG_Y)
            }
            Direction::Down => {
                local.y -= 1;
                self.chunk
            }
            Direction::Up if local.y == CHUNK_SIZE as u32 - 1 => {
                local.y = 0;
                self.chunk.offset(IVec3::Y)
            }
            Direction::Up => {
                local.y += 1;
                self.chunk
            }
            Direction::Forward if local.z == 0 => {
                local.z = CHUNK_SIZE as u32 - 1;
                self.chunk.offset(IVec3::NEG_Z)
            }
            Direction::Forward => {
                local.z -= 1;
                self.chunk
            }
            Direction::Backward if local.z == CHUNK_SIZE as u32 - 1 => {
                local.z = 0;
                self.chunk.offset(IVec3::Z)
            }
            Direction::Backward => {
                local.z += 1;
                self.chunk
            }
        };
        Self::new(chunk, LocalBlockPos::new(local.x, local.y, local.z))
    }
}

/// A Y-fast index into the canonical 16^3 chunk storage layout.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkIndex(u16);

impl ChunkIndex {
    pub const FIRST: Self = Self(0);
    pub const LAST: Self = Self(CHUNK_VOLUME as u16 - 1);

    pub const fn from_local(local: LocalBlockPos) -> Self {
        Self((local.y() + CHUNK_SIZE * (local.z() + CHUNK_SIZE * local.x())) as u16)
    }

    pub const fn try_from_usize(index: usize) -> Option<Self> {
        if index < CHUNK_VOLUME {
            Some(Self(index as u16))
        } else {
            None
        }
    }

    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }

    pub const fn local(self) -> LocalBlockPos {
        let index = self.as_usize();
        let x = index / (CHUNK_SIZE * CHUNK_SIZE);
        let in_slice = index % (CHUNK_SIZE * CHUNK_SIZE);
        let z = in_slice / CHUNK_SIZE;
        let y = in_slice % CHUNK_SIZE;
        LocalBlockPos::new(x as u32, y as u32, z as u32)
    }

    pub fn iter() -> impl ExactSizeIterator<Item = Self> {
        (0..CHUNK_VOLUME).map(|index| Self(index as u16))
    }
}

impl From<ChunkIndex> for usize {
    fn from(value: ChunkIndex) -> Self {
        value.as_usize()
    }
}

/// Compatibility helper for callers that iterate separate coordinates.
#[inline(always)]
pub const fn chunk_linear_index(x: usize, y: usize, z: usize) -> usize {
    LocalBlockPos::new(x as u32, y as u32, z as u32)
        .index()
        .as_usize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_positions_split_with_euclidean_negative_coordinates() {
        for value in [-17, -16, -15, -1, 0, 1, 15, 16, 17] {
            let address = WorldBlockPos::new(value, value, value).split();
            assert_eq!(address.world(), WorldBlockPos::new(value, value, value));
            assert!((0..CHUNK_ISIZE).contains(&(address.local().x() as i32)));
            assert!((0..CHUNK_ISIZE).contains(&(address.local().y() as i32)));
            assert!((0..CHUNK_ISIZE).contains(&(address.local().z() as i32)));
        }

        let negative_one = WorldBlockPos::new(-1, -1, -1).split();
        assert_eq!(negative_one.chunk(), ChunkPos::new(-1, -1, -1));
        assert_eq!(negative_one.local(), LocalBlockPos::new(15, 15, 15));
    }

    #[test]
    fn offset_normalizes_across_chunk_boundaries() {
        let origin = ChunkPos::ZERO.block(LocalBlockPos::ZERO);
        let previous = origin.offset(IVec3::NEG_X + IVec3::NEG_Y + IVec3::NEG_Z);
        assert_eq!(previous.chunk(), ChunkPos::new(-1, -1, -1));
        assert_eq!(previous.local(), LocalBlockPos::MAX);

        let next = ChunkPos::ZERO.block(LocalBlockPos::MAX).offset(IVec3::ONE);
        assert_eq!(next.chunk(), ChunkPos::new(1, 1, 1));
        assert_eq!(next.local(), LocalBlockPos::ZERO);
    }

    #[test]
    fn directional_neighbors_match_general_offsets_exhaustively() {
        let chunk = ChunkPos::new(-7, 3, -11);
        for index in ChunkIndex::iter() {
            let address = chunk.block(index.local());
            for direction in Direction::ALL {
                assert_eq!(
                    address.neighbor(direction),
                    address.offset(direction.offset())
                );
            }
        }
    }

    #[test]
    fn chunk_indices_round_trip_exhaustively() {
        for index in ChunkIndex::iter() {
            assert_eq!(index.local().index(), index);
        }

        assert_eq!(LocalBlockPos::new(0, 1, 0).index().as_usize(), 1);
        assert_eq!(LocalBlockPos::new(0, 0, 1).index().as_usize(), 16);
        assert_eq!(LocalBlockPos::new(1, 0, 0).index().as_usize(), 256);
        assert_eq!(ChunkIndex::LAST.local(), LocalBlockPos::MAX);
    }

    #[test]
    fn translation_is_floored_once_before_chunk_split() {
        let address = WorldBlockPos::from_translation(Vec3::new(-0.01, 16.99, -16.01)).split();
        assert_eq!(address.chunk(), ChunkPos::new(-1, 1, -2));
        assert_eq!(address.local(), LocalBlockPos::new(15, 0, 15));
    }

    #[test]
    fn chunk_columns_preserve_horizontal_coordinates_across_heights() {
        let column = ChunkColumn::from(ChunkPos::new(-7, 4, 11));

        assert_eq!(column.x(), -7);
        assert_eq!(column.z(), 11);
        assert_eq!(column.chunk(-3), ChunkPos::new(-7, -3, 11));
    }

    #[test]
    fn chunk_grid_arithmetic_preserves_coordinate_types() {
        let mut position = ChunkPos::new(-7, 4, 11);
        position += IVec3::new(2, -3, 5);
        assert_eq!(position, ChunkPos::new(-5, 1, 16));
        assert_eq!(position - IVec3::new(1, 2, 3), ChunkPos::new(-6, -1, 13));
        position -= IVec3::ONE;
        assert_eq!(position, ChunkPos::new(-6, 0, 15));
        assert_eq!(position - ChunkPos::new(-8, -2, 12), IVec3::new(2, 2, 3));

        let original = WorldBlockPos::new(-9, 3, 14);
        let displacement = IVec3::new(5, -7, 2);
        let mut world = original + displacement;
        assert_eq!(world, WorldBlockPos::new(-4, -4, 16));
        assert_eq!(world - original, displacement);
        world -= displacement;
        assert_eq!(world, original);
        world += IVec3::ONE;
        assert_eq!(world - IVec3::ONE, original);
    }

    #[test]
    fn semantic_chunk_coordinate_helpers_are_unambiguous() {
        let position = ChunkPos::new(-5, 9, 13);

        assert_eq!(position.with_y(-2), ChunkPos::new(-5, -2, 13));
        assert_eq!(position.column(), ChunkColumn::new(-5, 13));
        assert_eq!(
            position.with_y(-2),
            position.column().chunk(-2),
            "changing vertical position must preserve column identity"
        );
    }
}
