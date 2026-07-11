use bevy::platform::collections::{HashMap, HashSet};

use crate::quad::Direction;

use super::super::{Chunk, ChunkBlockPos, ChunkCell, ChunkPos};
use super::solver;
use super::storage::{ChunkHeightmap, ChunkLight};

const MAX_HEIGHT_CHUNKS: usize = (u8::MAX as usize + 1) / super::super::CHUNK_SIZE;

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

struct TargetChunk<'a> {
    chunk: &'a Chunk,
    original_light: &'a ChunkLight,
    original_heightmap: &'a ChunkHeightmap,
    light: ChunkLight,
    heightmap: ChunkHeightmap,
}

impl<'a> TargetChunk<'a> {
    fn new(chunk: &'a Chunk, light: &'a ChunkLight, heightmap: &'a ChunkHeightmap) -> Self {
        Self {
            chunk,
            original_light: light,
            original_heightmap: heightmap,
            light: ChunkLight::default(),
            heightmap: ChunkHeightmap::default(),
        }
    }
}

/// A self-contained lighting rebuild over a writable set of chunks.
///
/// Target membership defines the write boundary. Light from adjacent, loaded
/// chunks can enter the region through borrowed boundary data, but rebuilding
/// never mutates those boundary chunks.
pub struct ChunkLightRegion<'a> {
    height_chunks: usize,
    targets: HashMap<ChunkPos, TargetChunk<'a>>,
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
            targets: HashMap::new(),
            boundary_lights: HashMap::new(),
        }
    }

    pub fn insert_target(
        &mut self,
        position: ChunkPos,
        chunk: &'a Chunk,
        light: &'a ChunkLight,
        heightmap: &'a ChunkHeightmap,
    ) {
        let y = position.as_ivec3().y;
        assert!(
            (0..self.height_chunks as i32).contains(&y),
            "chunk lighting target must be inside the configured vertical range"
        );
        assert!(
            !self.targets.contains_key(&position),
            "chunk lighting target inserted more than once"
        );
        self.boundary_lights.remove(&position);
        self.targets
            .insert(position, TargetChunk::new(chunk, light, heightmap));
    }

    pub fn insert_boundary_light(&mut self, position: ChunkPos, light: &'a ChunkLight) {
        if !self.targets.contains_key(&position) {
            self.boundary_lights.insert(position, light);
        }
    }

    /// Returns the exact face-adjacent, non-target positions read by a rebuild.
    pub fn required_boundary_positions(&self) -> HashSet<ChunkPos> {
        let mut positions = HashSet::new();
        for &target in self.targets.keys() {
            for direction in Direction::ALL {
                let neighbor = target.offset(direction.offset());
                if !self.targets.contains_key(&neighbor) {
                    positions.insert(neighbor);
                }
            }
        }
        positions
    }

    pub fn rebuild(mut self) -> Vec<RebuiltChunkLight> {
        if !self.targets.is_empty() {
            solver::rebuild(&mut self);
        }

        let mut rebuilt = self
            .targets
            .into_iter()
            .map(|(position, target)| RebuiltChunkLight {
                position,
                light_changed: target.light != *target.original_light,
                heightmap_changed: target.heightmap != *target.original_heightmap,
                light: target.light,
                heightmap: target.heightmap,
            })
            .collect::<Vec<_>>();
        rebuilt.sort_unstable_by_key(|result| {
            let position = result.position.as_ivec3();
            (position.x, position.y, position.z)
        });
        rebuilt
    }

    pub(super) const fn height_chunks(&self) -> usize {
        self.height_chunks
    }

    pub(super) fn target_positions(&self) -> Vec<ChunkPos> {
        self.targets.keys().copied().collect()
    }

    pub(super) fn contains_target(&self, position: ChunkPos) -> bool {
        self.targets.contains_key(&position)
    }

    pub(super) fn target_chunk(&self, position: ChunkPos) -> Option<&'a Chunk> {
        self.targets.get(&position).map(|target| target.chunk)
    }

    pub(super) fn set_height(&mut self, position: ChunkPos, x: usize, z: usize, height: u8) {
        if let Some(target) = self.targets.get_mut(&position) {
            target.heightmap.heights[x][z] = height;
        }
    }

    pub(super) fn cell(&self, address: ChunkBlockPos) -> ChunkCell {
        self.target_chunk(address.chunk())
            .map(|chunk| chunk.cell(address.local()))
            .unwrap_or(ChunkCell::EMPTY)
    }

    pub(super) fn sky_light(&self, address: ChunkBlockPos) -> u8 {
        if let Some(target) = self.targets.get(&address.chunk()) {
            target.light.sky_light(address.local())
        } else {
            self.boundary_lights
                .get(&address.chunk())
                .map(|light| light.sky_light(address.local()))
                .unwrap_or(0)
        }
    }

    pub(super) fn block_light(&self, address: ChunkBlockPos) -> u8 {
        if let Some(target) = self.targets.get(&address.chunk()) {
            target.light.block_light(address.local())
        } else {
            self.boundary_lights
                .get(&address.chunk())
                .map(|light| light.block_light(address.local()))
                .unwrap_or(0)
        }
    }

    pub(super) fn write_sky_light(&mut self, address: ChunkBlockPos, value: u8) -> bool {
        let Some(target) = self.targets.get_mut(&address.chunk()) else {
            return false;
        };
        target.light.set_sky_light(address.local(), value);
        true
    }

    pub(super) fn write_block_light(&mut self, address: ChunkBlockPos, value: u8) -> bool {
        let Some(target) = self.targets.get_mut(&address.chunk()) else {
            return false;
        };
        target.light.set_block_light(address.local(), value);
        true
    }
}
