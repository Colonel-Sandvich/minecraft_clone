use bevy::platform::collections::{HashMap, HashSet};

use crate::quad::Direction;

use super::super::{Chunk, ChunkBlockPos, ChunkCell, ChunkPos};
use super::solver;
use super::storage::{ChunkHeightmap, ChunkLight};

const MAX_HEIGHT_CHUNKS: usize = (u8::MAX as usize + 1) / super::super::CHUNK_SIZE;
const MAX_DENSE_LOOKUP_AMPLIFICATION: usize = 8;
const VACANT_CALCULATION_INDEX: usize = usize::MAX;

/// The computed lighting state for one writable chunk in a rebuilt region.
#[derive(Debug)]
pub struct RebuiltChunkLight {
    pub position: ChunkPos,
    pub light: ChunkLight,
    pub heightmap: ChunkHeightmap,
    light_changed: bool,
    heightmap_changed: bool,
}

impl RebuiltChunkLight {
    pub const fn light_changed(&self) -> bool {
        self.light_changed
    }

    pub const fn heightmap_changed(&self) -> bool {
        self.heightmap_changed
    }
}

struct CalculationChunk<'a> {
    chunk: &'a Chunk,
    light: ChunkLight,
    heightmap: ChunkHeightmap,
}

impl<'a> CalculationChunk<'a> {
    fn new(chunk: &'a Chunk) -> Self {
        Self {
            chunk,
            light: ChunkLight::default(),
            heightmap: ChunkHeightmap::default(),
        }
    }
}

struct PreparedCalculationChunks<'a> {
    entries: Vec<(ChunkPos, CalculationChunk<'a>)>,
    lookup: CalculationLookup,
}

impl<'a> PreparedCalculationChunks<'a> {
    fn new(chunks: HashMap<ChunkPos, CalculationChunk<'a>>, height_chunks: usize) -> Self {
        let entries = chunks.into_iter().collect::<Vec<_>>();
        let lookup = CalculationLookup::new(&entries, height_chunks);
        Self { entries, lookup }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn positions(&self) -> impl ExactSizeIterator<Item = ChunkPos> + '_ {
        self.entries.iter().map(|(position, _)| *position)
    }

    fn get(&self, position: ChunkPos) -> Option<&CalculationChunk<'a>> {
        let index = self.lookup.index(position)?;
        Some(&self.entries[index].1)
    }

    fn get_mut(&mut self, position: ChunkPos) -> Option<&mut CalculationChunk<'a>> {
        let index = self.lookup.index(position)?;
        Some(&mut self.entries[index].1)
    }

    fn into_entries(self) -> impl Iterator<Item = (ChunkPos, CalculationChunk<'a>)> {
        self.entries.into_iter()
    }
}

enum CalculationLookup {
    Dense(DenseCalculationLookup),
    Sparse(HashMap<ChunkPos, usize>),
}

impl CalculationLookup {
    fn new<'a>(entries: &[(ChunkPos, CalculationChunk<'a>)], height_chunks: usize) -> Self {
        let Some((&(first, _), remaining)) = entries.split_first() else {
            return Self::Sparse(HashMap::new());
        };

        let (mut min_x, mut max_x) = (first.x(), first.x());
        let (mut min_z, mut max_z) = (first.z(), first.z());
        for (position, _) in remaining {
            min_x = min_x.min(position.x());
            max_x = max_x.max(position.x());
            min_z = min_z.min(position.z());
            max_z = max_z.max(position.z());
        }

        let width = usize::try_from(i64::from(max_x) - i64::from(min_x) + 1).ok();
        let depth = usize::try_from(i64::from(max_z) - i64::from(min_z) + 1).ok();
        let slot_count = width
            .zip(depth)
            .and_then(|(width, depth)| width.checked_mul(depth))
            .and_then(|columns| columns.checked_mul(height_chunks));
        let dense_limit = entries.len().saturating_mul(MAX_DENSE_LOOKUP_AMPLIFICATION);

        if let (Some(width), Some(depth), Some(slot_count)) = (width, depth, slot_count)
            && slot_count <= dense_limit
        {
            let mut dense = DenseCalculationLookup {
                min_x,
                min_z,
                width,
                depth,
                height: height_chunks,
                indices: vec![VACANT_CALCULATION_INDEX; slot_count],
            };
            for (index, (position, _)) in entries.iter().enumerate() {
                let slot = dense
                    .slot(*position)
                    .expect("calculation bounds must contain every input chunk");
                dense.indices[slot] = index;
            }
            return Self::Dense(dense);
        }

        Self::Sparse(
            entries
                .iter()
                .enumerate()
                .map(|(index, (position, _))| (*position, index))
                .collect(),
        )
    }

    fn index(&self, position: ChunkPos) -> Option<usize> {
        match self {
            Self::Dense(dense) => dense.index(position),
            Self::Sparse(indices) => indices.get(&position).copied(),
        }
    }
}

struct DenseCalculationLookup {
    min_x: i32,
    min_z: i32,
    width: usize,
    depth: usize,
    height: usize,
    indices: Vec<usize>,
}

impl DenseCalculationLookup {
    fn slot(&self, position: ChunkPos) -> Option<usize> {
        let x = usize::try_from(i64::from(position.x()) - i64::from(self.min_x)).ok()?;
        let y = usize::try_from(position.y()).ok()?;
        let z = usize::try_from(i64::from(position.z()) - i64::from(self.min_z)).ok()?;
        if x >= self.width || y >= self.height || z >= self.depth {
            return None;
        }
        Some((z * self.width + x) * self.height + y)
    }

    fn index(&self, position: ChunkPos) -> Option<usize> {
        let index = self.indices[self.slot(position)?];
        (index != VACANT_CALCULATION_INDEX).then_some(index)
    }
}

#[derive(Clone, Copy)]
struct CommitBaseline<'a> {
    light: &'a ChunkLight,
    heightmap: &'a ChunkHeightmap,
}

#[derive(Debug, Clone, Copy)]
struct CommitChanges {
    light_changed: bool,
    heightmap_changed: bool,
}

#[derive(Debug)]
struct SolvedChunkLight {
    light: ChunkLight,
    heightmap: ChunkHeightmap,
}

/// Owned lighting calculated across a region, including scratch dependencies.
#[derive(Debug)]
pub struct SolvedChunkLightRegion {
    chunks: HashMap<ChunkPos, SolvedChunkLight>,
    commit_changes: HashMap<ChunkPos, CommitChanges>,
}

impl SolvedChunkLightRegion {
    pub fn light(&self, position: ChunkPos) -> Option<&ChunkLight> {
        self.chunks.get(&position).map(|chunk| &chunk.light)
    }

    pub fn lights(&self) -> impl ExactSizeIterator<Item = (ChunkPos, &ChunkLight)> {
        self.chunks
            .iter()
            .map(|(&position, chunk)| (position, &chunk.light))
    }

    pub fn heightmap(&self, position: ChunkPos) -> Option<&ChunkHeightmap> {
        self.chunks.get(&position).map(|chunk| &chunk.heightmap)
    }

    pub fn into_committed(mut self) -> Vec<RebuiltChunkLight> {
        let mut rebuilt = self
            .commit_changes
            .into_iter()
            .map(|(position, changes)| {
                let chunk = self
                    .chunks
                    .remove(&position)
                    .expect("committed lighting result must have been calculated");
                RebuiltChunkLight {
                    position,
                    light: chunk.light,
                    heightmap: chunk.heightmap,
                    light_changed: changes.light_changed,
                    heightmap_changed: changes.heightmap_changed,
                }
            })
            .collect::<Vec<_>>();
        rebuilt.sort_unstable_by_key(|result| {
            let position = result.position.as_ivec3();
            (position.x, position.y, position.z)
        });
        rebuilt
    }
}

/// A self-contained lighting rebuild over a writable set of chunks.
///
/// Calculation membership defines the solver's read/write boundary. A subset
/// can be marked for commit; other calculation chunks are scratch dependencies.
/// Light from adjacent, loaded chunks can enter through borrowed boundary data,
/// but rebuilding never mutates those boundary chunks.
pub struct ChunkLightRegion<'a> {
    height_chunks: usize,
    calculation_chunks: HashMap<ChunkPos, CalculationChunk<'a>>,
    commit_baselines: HashMap<ChunkPos, CommitBaseline<'a>>,
    boundary_lights: HashMap<ChunkPos, &'a ChunkLight>,
}

pub(super) struct PreparedChunkLightRegion<'a> {
    height_chunks: usize,
    calculation_chunks: PreparedCalculationChunks<'a>,
    boundary_lights: HashMap<ChunkPos, &'a ChunkLight>,
}

impl<'a> ChunkLightRegion<'a> {
    pub fn new(height_chunks: usize) -> Self {
        assert!(
            height_chunks <= MAX_HEIGHT_CHUNKS,
            "chunk lighting height must fit the u8 heightmap"
        );
        Self {
            height_chunks,
            calculation_chunks: HashMap::new(),
            commit_baselines: HashMap::new(),
            boundary_lights: HashMap::new(),
        }
    }

    /// Adds a writable calculation chunk without making it a committed output.
    pub fn insert_calculation_chunk(&mut self, position: ChunkPos, chunk: &'a Chunk) {
        let y = position.as_ivec3().y;
        assert!(
            (0..self.height_chunks as i32).contains(&y),
            "chunk lighting calculation must be inside the configured vertical range"
        );
        assert!(
            !self.calculation_chunks.contains_key(&position),
            "chunk lighting calculation inserted more than once"
        );
        self.boundary_lights.remove(&position);
        self.calculation_chunks
            .insert(position, CalculationChunk::new(chunk));
    }

    /// Marks an inserted calculation chunk as an output that should be committed.
    pub fn mark_commit_target(
        &mut self,
        position: ChunkPos,
        light: &'a ChunkLight,
        heightmap: &'a ChunkHeightmap,
    ) {
        assert!(
            self.calculation_chunks.contains_key(&position),
            "chunk lighting commit target must first be inserted for calculation"
        );
        assert!(
            !self.commit_baselines.contains_key(&position),
            "chunk lighting commit target marked more than once"
        );
        self.commit_baselines
            .insert(position, CommitBaseline { light, heightmap });
    }

    /// Adds a calculation chunk and marks it for commit.
    pub fn insert_target(
        &mut self,
        position: ChunkPos,
        chunk: &'a Chunk,
        light: &'a ChunkLight,
        heightmap: &'a ChunkHeightmap,
    ) {
        self.insert_calculation_chunk(position, chunk);
        self.mark_commit_target(position, light, heightmap);
    }

    pub fn insert_boundary_light(&mut self, position: ChunkPos, light: &'a ChunkLight) {
        if !self.calculation_chunks.contains_key(&position) {
            self.boundary_lights.insert(position, light);
        }
    }

    /// Returns the exact face-adjacent positions outside the calculation region.
    pub fn required_boundary_positions(&self) -> HashSet<ChunkPos> {
        let mut positions = HashSet::new();
        for &target in self.calculation_chunks.keys() {
            for direction in Direction::ALL {
                let neighbor = target.offset(direction.offset());
                if !self.calculation_chunks.contains_key(&neighbor) {
                    positions.insert(neighbor);
                }
            }
        }
        positions
    }

    pub fn solve(self) -> SolvedChunkLightRegion {
        let Self {
            height_chunks,
            calculation_chunks,
            commit_baselines,
            boundary_lights,
        } = self;
        let mut prepared = PreparedChunkLightRegion {
            height_chunks,
            calculation_chunks: PreparedCalculationChunks::new(calculation_chunks, height_chunks),
            boundary_lights,
        };
        if !prepared.calculation_chunks.is_empty() {
            solver::rebuild(&mut prepared);
        }

        let commit_changes = commit_baselines
            .iter()
            .map(|(&position, baseline)| {
                let calculated = prepared
                    .calculation_chunks
                    .get(position)
                    .expect("commit target must remain in the calculation region");
                (
                    position,
                    CommitChanges {
                        light_changed: calculated.light != *baseline.light,
                        heightmap_changed: calculated.heightmap != *baseline.heightmap,
                    },
                )
            })
            .collect();
        let chunks = prepared
            .calculation_chunks
            .into_entries()
            .map(|(position, calculated)| {
                (
                    position,
                    SolvedChunkLight {
                        light: calculated.light,
                        heightmap: calculated.heightmap,
                    },
                )
            })
            .collect();

        SolvedChunkLightRegion {
            chunks,
            commit_changes,
        }
    }

    pub fn rebuild(self) -> Vec<RebuiltChunkLight> {
        self.solve().into_committed()
    }
}

impl<'a> PreparedChunkLightRegion<'a> {
    pub(super) const fn height_chunks(&self) -> usize {
        self.height_chunks
    }

    pub(super) fn calculation_positions(&self) -> Vec<ChunkPos> {
        self.calculation_chunks.positions().collect()
    }

    pub(super) fn contains_calculation(&self, position: ChunkPos) -> bool {
        self.calculation_chunks.get(position).is_some()
    }

    pub(super) fn calculation_chunk(&self, position: ChunkPos) -> Option<&'a Chunk> {
        self.calculation_chunks
            .get(position)
            .map(|calculation| calculation.chunk)
    }

    pub(super) fn set_height(&mut self, position: ChunkPos, x: usize, z: usize, height: u8) {
        if let Some(calculation) = self.calculation_chunks.get_mut(position) {
            calculation.heightmap.heights[x][z] = height;
        }
    }

    pub(super) fn cell(&self, address: ChunkBlockPos) -> ChunkCell {
        self.calculation_chunk(address.chunk())
            .map(|chunk| chunk.cell(address.local()))
            .unwrap_or(ChunkCell::EMPTY)
    }

    pub(super) fn sky_light(&self, address: ChunkBlockPos) -> u8 {
        if let Some(calculation) = self.calculation_chunks.get(address.chunk()) {
            calculation.light.sky_light(address.local())
        } else {
            self.boundary_lights
                .get(&address.chunk())
                .map(|light| light.sky_light(address.local()))
                .unwrap_or(0)
        }
    }

    pub(super) fn block_light(&self, address: ChunkBlockPos) -> u8 {
        if let Some(calculation) = self.calculation_chunks.get(address.chunk()) {
            calculation.light.block_light(address.local())
        } else {
            self.boundary_lights
                .get(&address.chunk())
                .map(|light| light.block_light(address.local()))
                .unwrap_or(0)
        }
    }

    pub(super) fn write_sky_light(&mut self, address: ChunkBlockPos, value: u8) -> bool {
        let Some(calculation) = self.calculation_chunks.get_mut(address.chunk()) else {
            return false;
        };
        calculation.light.set_sky_light(address.local(), value);
        true
    }

    pub(super) fn write_block_light(&mut self, address: ChunkBlockPos, value: u8) -> bool {
        let Some(calculation) = self.calculation_chunks.get_mut(address.chunk()) else {
            return false;
        };
        calculation.light.set_block_light(address.local(), value);
        true
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[test]
    fn dense_lookup_distinguishes_holes_and_out_of_bounds_positions() {
        let first_chunk = Chunk::default();
        let second_chunk = Chunk::default();
        let first = ChunkPos::new(-2, 0, -3);
        let second = ChunkPos::new(0, 1, -2);
        let entries = vec![
            (first, CalculationChunk::new(&first_chunk)),
            (second, CalculationChunk::new(&second_chunk)),
        ];

        let lookup = CalculationLookup::new(&entries, 2);
        assert!(matches!(&lookup, CalculationLookup::Dense(_)));
        assert_eq!(lookup.index(first), Some(0));
        assert_eq!(lookup.index(second), Some(1));
        assert_eq!(lookup.index(ChunkPos::new(-1, 0, -3)), None);
        assert_eq!(lookup.index(ChunkPos::new(-3, 0, -3)), None);
        assert_eq!(lookup.index(ChunkPos::new(-2, -1, -3)), None);
        assert_eq!(lookup.index(ChunkPos::new(-2, 2, -3)), None);
        assert_eq!(lookup.index(ChunkPos::new(-2, 0, -4)), None);
    }

    #[test]
    fn sparse_lookup_avoids_allocating_a_disconnected_coordinate_span() {
        let first_chunk = Chunk::default();
        let second_chunk = Chunk::default();
        let first = ChunkPos::new(-10_000, 0, -10_000);
        let second = ChunkPos::new(10_000, 0, 10_000);
        let entries = vec![
            (first, CalculationChunk::new(&first_chunk)),
            (second, CalculationChunk::new(&second_chunk)),
        ];

        let lookup = CalculationLookup::new(&entries, 1);
        assert!(matches!(&lookup, CalculationLookup::Sparse(_)));
        assert_eq!(lookup.index(first), Some(0));
        assert_eq!(lookup.index(second), Some(1));
        assert_eq!(lookup.index(ChunkPos::ZERO), None);
    }
}
