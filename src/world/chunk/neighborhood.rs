use std::ops::Range;

use bevy::math::IVec3;
#[cfg(test)]
use bevy::math::UVec3;

use crate::quad::Direction;

use super::coords::{CHUNK_ISIZE, CHUNK_SIZE, LocalBlockPos};

pub(crate) const PADDED_CHUNK_SIZE: usize = CHUNK_SIZE + 2;
pub(crate) const PADDED_CHUNK_LAYER_SIZE: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const PADDED_CHUNK_VOLUME: usize =
    PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;

/// One of the 26 adjacent chunk-grid offsets.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NeighborOffset(IVec3);

impl NeighborOffset {
    pub(crate) const fn try_new(offset: IVec3) -> Option<Self> {
        if offset.x >= -1
            && offset.x <= 1
            && offset.y >= -1
            && offset.y <= 1
            && offset.z >= -1
            && offset.z <= 1
            && (offset.x != 0 || offset.y != 0 || offset.z != 0)
        {
            Some(Self(offset))
        } else {
            None
        }
    }

    pub(crate) const fn as_ivec3(self) -> IVec3 {
        self.0
    }

    pub(crate) fn all() -> impl Iterator<Item = Self> {
        (-1..=1).flat_map(|x| {
            (-1..=1)
                .flat_map(move |y| (-1..=1).filter_map(move |z| Self::try_new(IVec3::new(x, y, z))))
        })
    }

    pub(crate) fn touching(local: LocalBlockPos) -> impl Iterator<Item = Self> {
        Self::all().filter(move |offset| {
            neighbor_axis_touches_local(offset.0.x, local.x())
                && neighbor_axis_touches_local(offset.0.y, local.y())
                && neighbor_axis_touches_local(offset.0.z, local.z())
        })
    }

    pub(crate) fn source_axis_range(axis: i32) -> Range<usize> {
        match axis {
            -1 => CHUNK_SIZE - 1..CHUNK_SIZE,
            0 => 0..CHUNK_SIZE,
            1 => 0..1,
            _ => unreachable!("neighbor axis must be in -1..=1"),
        }
    }
}

fn neighbor_axis_touches_local(offset: i32, coordinate: usize) -> bool {
    match offset {
        -1 => coordinate == 0,
        0 => true,
        1 => coordinate == CHUNK_SIZE - 1,
        _ => false,
    }
}

/// An X-fast index into a chunk plus its one-cell halo.
#[repr(transparent)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PaddedChunkIndex(u16);

impl PaddedChunkIndex {
    #[cfg(test)]
    pub(crate) const FIRST: Self = Self(0);
    #[cfg(test)]
    pub(crate) const LAST: Self = Self(PADDED_CHUNK_VOLUME as u16 - 1);

    pub(crate) const fn new(x: usize, y: usize, z: usize) -> Self {
        assert!(x < PADDED_CHUNK_SIZE);
        assert!(y < PADDED_CHUNK_SIZE);
        assert!(z < PADDED_CHUNK_SIZE);
        Self((x + PADDED_CHUNK_SIZE * (z + PADDED_CHUNK_SIZE * y)) as u16)
    }

    pub(crate) const fn from_local(local: LocalBlockPos) -> Self {
        Self::new(local.x() + 1, local.y() + 1, local.z() + 1)
    }

    pub(crate) const fn from_relative(relative: IVec3) -> Option<Self> {
        if relative.x < -1
            || relative.x > CHUNK_ISIZE
            || relative.y < -1
            || relative.y > CHUNK_ISIZE
            || relative.z < -1
            || relative.z > CHUNK_ISIZE
        {
            return None;
        }
        Some(Self::new(
            (relative.x + 1) as usize,
            (relative.y + 1) as usize,
            (relative.z + 1) as usize,
        ))
    }

    pub(crate) const fn as_usize(self) -> usize {
        self.0 as usize
    }

    #[cfg(test)]
    pub(crate) const fn coordinates(self) -> UVec3 {
        let index = self.as_usize();
        let y = index / PADDED_CHUNK_LAYER_SIZE;
        let in_layer = index % PADDED_CHUNK_LAYER_SIZE;
        let z = in_layer / PADDED_CHUNK_SIZE;
        let x = in_layer % PADDED_CHUNK_SIZE;
        UVec3::new(x as u32, y as u32, z as u32)
    }

    #[cfg(test)]
    pub(crate) fn neighbor(self, direction: Direction) -> Option<Self> {
        let pos = self.coordinates().as_ivec3();
        let offset = match direction {
            Direction::Left => IVec3::NEG_X,
            Direction::Right => IVec3::X,
            Direction::Down => IVec3::NEG_Y,
            Direction::Up => IVec3::Y,
            Direction::Forward => IVec3::NEG_Z,
            Direction::Backward => IVec3::Z,
        };
        let neighbor = pos + offset;
        if neighbor.cmplt(IVec3::ZERO).any()
            || neighbor.cmpge(IVec3::splat(PADDED_CHUNK_SIZE as i32)).any()
        {
            return None;
        }
        Some(Self::new(
            neighbor.x as usize,
            neighbor.y as usize,
            neighbor.z as usize,
        ))
    }
}

impl From<PaddedChunkIndex> for usize {
    fn from(value: PaddedChunkIndex) -> Self {
        value.as_usize()
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PaddedChunkOffset(isize);

impl PaddedChunkOffset {
    pub(crate) const fn for_direction(direction: Direction) -> Self {
        Self(match direction {
            Direction::Left => -1,
            Direction::Right => 1,
            Direction::Down => -(PADDED_CHUNK_LAYER_SIZE as isize),
            Direction::Up => PADDED_CHUNK_LAYER_SIZE as isize,
            Direction::Forward => -(PADDED_CHUNK_SIZE as isize),
            Direction::Backward => PADDED_CHUNK_SIZE as isize,
        })
    }

    pub(crate) const fn as_isize(self) -> isize {
        self.0
    }
}

#[inline(always)]
pub(crate) const fn padded_chunk_index(x: usize, y: usize, z: usize) -> usize {
    PaddedChunkIndex::new(x, y, z).as_usize()
}

pub(crate) fn chunk_neighbor_offsets() -> impl Iterator<Item = IVec3> {
    NeighborOffset::all().map(NeighborOffset::as_ivec3)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn neighbor_offsets_cover_faces_edges_and_corners() {
        let offsets = NeighborOffset::all().collect::<Vec<_>>();
        assert_eq!(offsets.len(), 26);
        assert_eq!(offsets.iter().copied().collect::<HashSet<_>>().len(), 26);
        assert!(!offsets.iter().any(|offset| offset.0 == IVec3::ZERO));

        assert_eq!(
            NeighborOffset::touching(LocalBlockPos::new(1, 2, 3)).count(),
            0
        );
        assert_eq!(
            NeighborOffset::touching(LocalBlockPos::new(0, 2, 3))
                .map(NeighborOffset::as_ivec3)
                .collect::<Vec<_>>(),
            vec![IVec3::NEG_X]
        );
        assert_eq!(
            NeighborOffset::touching(LocalBlockPos::new(15, 2, 3))
                .map(NeighborOffset::as_ivec3)
                .collect::<Vec<_>>(),
            vec![IVec3::X]
        );

        let edge_offsets = NeighborOffset::touching(LocalBlockPos::new(0, 0, 3))
            .map(NeighborOffset::as_ivec3)
            .collect::<HashSet<_>>();
        assert_eq!(
            edge_offsets,
            HashSet::from([IVec3::NEG_X, IVec3::NEG_Y, IVec3::new(-1, -1, 0)])
        );
        assert_eq!(NeighborOffset::touching(LocalBlockPos::ZERO).count(), 7);
    }

    #[test]
    fn padded_indices_round_trip_and_keep_x_fast_layout() {
        for index in 0..PADDED_CHUNK_VOLUME {
            let typed = PaddedChunkIndex(index as u16);
            let pos = typed.coordinates();
            assert_eq!(
                PaddedChunkIndex::new(pos.x as usize, pos.y as usize, pos.z as usize),
                typed
            );
        }

        assert_eq!(PaddedChunkIndex::new(1, 0, 0).as_usize(), 1);
        assert_eq!(PaddedChunkIndex::new(0, 0, 1).as_usize(), 18);
        assert_eq!(PaddedChunkIndex::new(0, 1, 0).as_usize(), 18 * 18);
        assert_eq!(
            PaddedChunkIndex::from_relative(IVec3::splat(-1)),
            Some(PaddedChunkIndex::FIRST)
        );
        assert_eq!(
            PaddedChunkIndex::from_relative(IVec3::splat(CHUNK_ISIZE)),
            Some(PaddedChunkIndex::LAST)
        );
    }

    #[test]
    fn direction_offsets_reach_expected_neighbors() {
        let center = PaddedChunkIndex::from_local(LocalBlockPos::new(8, 8, 8));
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Down,
            Direction::Up,
            Direction::Forward,
            Direction::Backward,
        ] {
            let actual = center.neighbor(direction).unwrap().coordinates().as_ivec3();
            let direction_vector = match direction {
                Direction::Left => IVec3::NEG_X,
                Direction::Right => IVec3::X,
                Direction::Down => IVec3::NEG_Y,
                Direction::Up => IVec3::Y,
                Direction::Forward => IVec3::NEG_Z,
                Direction::Backward => IVec3::Z,
            };
            let expected = center.coordinates().as_ivec3() + direction_vector;
            assert_eq!(actual, expected, "{direction:?}");
        }
    }

    #[test]
    fn shader_uses_the_cpu_padded_chunk_dimension() {
        let expected = format!("const PADDED_DIM: u32 = {PADDED_CHUNK_SIZE}u;");
        assert!(
            crate::world::chunk::mesh::TERRAIN_SHADER_SOURCE.contains(&expected),
            "terrain shader must use the CPU padded chunk dimension"
        );
        assert!(
            crate::world::chunk::mesh::TERRAIN_SHADER_SOURCE
                .contains("ilp.x + ilp.z * PADDED_DIM + ilp.y * PADDED_AREA"),
            "terrain shader must use the shared X-fast padded layout"
        );
    }
}
