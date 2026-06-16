use std::collections::VecDeque;

use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
};
use serde::{Deserialize, Serialize};

use crate::block::BlockType;

use super::{CHUNK_ISIZE, CHUNK_SIZE, Chunk, chunk_neighbor_offsets};

const SKY_LIGHT_MAX: u8 = 15;

const PADDED_CHUNK_SIZE: usize = CHUNK_SIZE + 2;
const PADDED_CHUNK_LAYER_SIZE: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
const PADDED_CHUNK_VOLUME: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
const PADDED_LIGHT_WORDS: usize = PADDED_CHUNK_VOLUME.div_ceil(4);
const MISSING_PADDED_LIGHT_WORD: u32 = 0xF0F0F0F0;

/// Direction order: pairs at indices (0,1), (2,3), (4,5) are opposites.
/// Opposite of idx = idx ^ 1.
const DIRECTION_OFFSETS: [IVec3; 6] = [
    IVec3::X,
    IVec3::NEG_X,
    IVec3::Z,
    IVec3::NEG_Z,
    IVec3::Y,
    IVec3::NEG_Y,
];
const ALL_DIRECTIONS_BITSET: u8 = 0b111111;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ChunkLight {
    pub light: [[[u8; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
}

impl Default for ChunkLight {
    fn default() -> Self {
        Self {
            light: [[[0u8; CHUNK_SIZE]; CHUNK_SIZE]; CHUNK_SIZE],
        }
    }
}

fn source_range_for_neighbor_axis(delta: i32) -> std::ops::Range<usize> {
    match delta {
        -1 => CHUNK_SIZE - 1..CHUNK_SIZE,
        0 => 0..CHUNK_SIZE,
        1 => 0..1,
        _ => unreachable!("invalid neighbor offset"),
    }
}

fn face_local_pair(offset: IVec3, a: usize, b: usize) -> Option<(UVec3, UVec3)> {
    let a = a as u32;
    let b = b as u32;
    Some(match (offset.x, offset.y, offset.z) {
        (-1, 0, 0) => (uvec3(0, a, b), uvec3(15, a, b)),
        (1, 0, 0) => (uvec3(15, a, b), uvec3(0, a, b)),
        (0, -1, 0) => (uvec3(a, 0, b), uvec3(a, 15, b)),
        (0, 1, 0) => (uvec3(a, 15, b), uvec3(a, 0, b)),
        (0, 0, -1) => (uvec3(a, b, 0), uvec3(a, b, 15)),
        (0, 0, 1) => (uvec3(a, b, 15), uvec3(a, b, 0)),
        _ => return None,
    })
}

impl ChunkLight {
    pub fn sky_light(&self, pos: UVec3) -> u8 {
        let packed = self.light[pos.x as usize][pos.z as usize][pos.y as usize];
        packed >> 4
    }

    pub fn block_light(&self, pos: UVec3) -> u8 {
        let packed = self.light[pos.x as usize][pos.z as usize][pos.y as usize];
        packed & 0x0F
    }

    pub fn packed_light(&self, pos: UVec3) -> u8 {
        self.light[pos.x as usize][pos.z as usize][pos.y as usize]
    }

    pub fn packed_light_at(&self, x: usize, z: usize, y: usize) -> u8 {
        self.light[x][z][y]
    }

    /// Build a padded 18³ light buffer for vertex pulling.
    ///
    /// `center_pos` is the chunk's position in chunk coords,
    /// `lights` is a map of all available chunks' light data (keyed by chunk position).
    /// Returns a flat array of u32 values, with four packed padded cells per word.
    /// Cell layout before packing: `index = x + z * 18 + y * 18 * 18`.
    pub fn build_padded_light_data(
        center_pos: IVec3,
        lights: &HashMap<IVec3, &ChunkLight>,
    ) -> Box<[u32]> {
        let mut data = vec![MISSING_PADDED_LIGHT_WORD; PADDED_LIGHT_WORDS].into_boxed_slice();

        // Copy center chunk (offset 0,0,0)
        if let Some(center) = lights.get(&center_pos) {
            for x in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for y in 0..CHUNK_SIZE {
                        let padded_idx = (x + 1)
                            + (z + 1) * PADDED_CHUNK_SIZE
                            + (y + 1) * PADDED_CHUNK_LAYER_SIZE;
                        write_padded_light(&mut data, padded_idx, center.packed_light_at(x, z, y));
                    }
                }
            }
        }

        // Copy neighbor chunks' border regions
        for offset in chunk_neighbor_offsets() {
            let Some(neighbor) = lights.get(&(center_pos + offset)) else {
                continue;
            };

            for x in source_range_for_neighbor_axis(offset.x) {
                for z in source_range_for_neighbor_axis(offset.z) {
                    for y in source_range_for_neighbor_axis(offset.y) {
                        let px = (x as i32 + offset.x * CHUNK_ISIZE) + 1;
                        let pz = (z as i32 + offset.z * CHUNK_ISIZE) + 1;
                        let py = (y as i32 + offset.y * CHUNK_ISIZE) + 1;
                        let padded_idx = px as usize
                            + pz as usize * PADDED_CHUNK_SIZE
                            + py as usize * PADDED_CHUNK_LAYER_SIZE;
                        write_padded_light(
                            &mut data,
                            padded_idx,
                            neighbor.packed_light_at(x, z, y),
                        );
                    }
                }
            }
        }

        data
    }

    pub fn set_sky_light(&mut self, pos: UVec3, value: u8) {
        let slot = &mut self.light[pos.x as usize][pos.z as usize][pos.y as usize];
        *slot = (*slot & 0x0F) | ((value & 0x0F) << 4);
    }

    pub fn set_block_light(&mut self, pos: UVec3, value: u8) {
        let slot = &mut self.light[pos.x as usize][pos.z as usize][pos.y as usize];
        *slot = (*slot & 0xF0) | (value & 0x0F);
    }

    pub fn set_packed_light(&mut self, pos: UVec3, value: u8) {
        self.light[pos.x as usize][pos.z as usize][pos.y as usize] = value;
    }

    pub fn reset_all_sky_light(&mut self) {
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    self.light[x][z][y] &= 0x0F;
                }
            }
        }
    }

    pub fn reset_all_block_light(&mut self) {
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    self.light[x][z][y] &= 0xF0;
                }
            }
        }
    }
}

fn write_padded_light(data: &mut [u32], padded_idx: usize, packed_light: u8) {
    let word_idx = padded_idx / 4;
    let shift = (padded_idx % 4) * 8;
    let mask = 0xFFu32 << shift;
    data[word_idx] = (data[word_idx] & !mask) | ((packed_light as u32) << shift);
}

#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkHeightmap {
    pub heights: [[u8; CHUNK_SIZE]; CHUNK_SIZE],
}

impl ChunkHeightmap {
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, bincode::config::standard())
            .expect("ChunkHeightmap serialization is infallible")
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        bincode::serde::decode_from_slice(bytes, bincode::config::standard())
            .map(|(hm, _)| hm)
            .unwrap_or_default()
    }
}

pub(crate) fn offset_to_bit_index(offset: IVec3) -> u32 {
    debug_assert!(
        offset != IVec3::ZERO && offset.x.abs() <= 1 && offset.y.abs() <= 1 && offset.z.abs() <= 1
    );
    let ix = (offset.x + 1) as u32;
    let iy = (offset.y + 1) as u32;
    let iz = (offset.z + 1) as u32;
    let flat = ix * 9 + iy * 3 + iz;
    if flat > 13 { flat - 1 } else { flat }
}

// ── Coordinate helpers ─────────────────────────────────────────────────────

fn block_at(
    center_chunk: &Chunk,
    chunks: &HashMap<IVec3, &Chunk>,
    center_pos: IVec3,
    chunk_pos: IVec3,
    local: UVec3,
) -> BlockType {
    if chunk_pos == center_pos {
        center_chunk.get_block(local)
    } else if let Some(chunk) = chunks.get(&chunk_pos) {
        chunk.get_block(local)
    } else {
        BlockType::Air
    }
}

// The &mut in the signature is intentional: callers hold &mut on the map
// for write_sky_light / write_block_light later, so a shared-reference
// re-borrow is needed here.
fn sky_light_at(
    center: &ChunkLight,
    neighbor_lights: &HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    chunk_pos: IVec3,
    local: UVec3,
) -> u8 {
    if chunk_pos == center_pos {
        center.sky_light(local)
    } else if let Some(light) = neighbor_lights.get(&chunk_pos) {
        light.sky_light(local)
    } else {
        SKY_LIGHT_MAX
    }
}

fn block_light_at(
    center: &ChunkLight,
    neighbor_lights: &HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    chunk_pos: IVec3,
    local: UVec3,
) -> u8 {
    if chunk_pos == center_pos {
        center.block_light(local)
    } else if let Some(light) = neighbor_lights.get(&chunk_pos) {
        light.block_light(local)
    } else {
        0
    }
}

fn write_sky_light(
    center: &mut ChunkLight,
    neighbor_lights: &mut HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    offset: IVec3,
    local: UVec3,
    value: u8,
    dirty_neighbors: &mut u32,
) -> bool {
    if offset == center_pos {
        center.set_sky_light(local, value);
        true
    } else if let Some(light) = neighbor_lights.get_mut(&offset) {
        light.set_sky_light(local, value);
        *dirty_neighbors |= 1 << offset_to_bit_index(offset);
        true
    } else {
        false
    }
}

fn write_block_light(
    center: &mut ChunkLight,
    neighbor_lights: &mut HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    offset: IVec3,
    local: UVec3,
    value: u8,
    dirty_neighbors: &mut u32,
) -> bool {
    if offset == center_pos {
        center.set_block_light(local, value);
        true
    } else if let Some(light) = neighbor_lights.get_mut(&offset) {
        light.set_block_light(local, value);
        *dirty_neighbors |= 1 << offset_to_bit_index(offset);
        true
    } else {
        false
    }
}

pub fn world_to_chunk_local(world: IVec3) -> (IVec3, UVec3) {
    let chunk = (world.as_vec3() / CHUNK_ISIZE as f32).floor().as_ivec3();
    let local = (world - chunk * CHUNK_ISIZE).as_uvec3();
    (chunk, local)
}

fn neighbor_chunk_local(chunk_pos: IVec3, local: UVec3, offset: IVec3) -> (IVec3, UVec3) {
    let mut chunk = chunk_pos;
    let mut local = local.as_ivec3() + offset;

    if local.x < 0 {
        chunk.x -= 1;
        local.x += CHUNK_ISIZE;
    } else if local.x >= CHUNK_ISIZE {
        chunk.x += 1;
        local.x -= CHUNK_ISIZE;
    }

    if local.y < 0 {
        chunk.y -= 1;
        local.y += CHUNK_ISIZE;
    } else if local.y >= CHUNK_ISIZE {
        chunk.y += 1;
        local.y -= CHUNK_ISIZE;
    }

    if local.z < 0 {
        chunk.z -= 1;
        local.z += CHUNK_ISIZE;
    } else if local.z >= CHUNK_ISIZE {
        chunk.z += 1;
        local.z -= CHUNK_ISIZE;
    }

    (chunk, local.as_uvec3())
}

// ── Starlight-style queue entries ───────────────────────────────────────────

struct IncreaseEntry {
    chunk: IVec3,
    local: UVec3,
    level: u8,
    /// Bitmask of direction indices to propagate to.
    directions: u8,
}

struct DecreaseEntry {
    chunk: IVec3,
    local: UVec3,
    level: u8,
    /// Bitmask of direction indices to propagate to.
    directions: u8,
}

// ── Starlight increase propagation ──────────────────────────────────────────
//
// Propagates light levels FORWARD to neighbors. Each queue entry stores the
// level being propagated and a direction bitset. For each direction in the
// bitset, calculates what the neighbor SHOULD receive (this_level - opacity)
// and if it's brighter than the neighbor's current level, writes it and
// enqueues the neighbor with bitset excluding the opposite direction.
//
// Key optimizations over vanilla BFS:
//   1. Forward propagation: each block checks its own level against neighbors
//      ONCE, vs vanilla polling ALL 6 neighbors per block (~6x fewer reads).
//   2. Short-circuit: `current >= propagated - 1` skips block reads for
//      already-lit neighbors (~6x fewer block reads).
//   3. Direction bitset excludes source direction (saves 1/6th).

fn propagate_sky_increase(
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    center_chunk: &Chunk,
    queue: &mut VecDeque<IncreaseEntry>,
    dirty_neighbors: &mut u32,
) {
    while let Some(entry) = queue.pop_front() {
        if entry.level <= 1 {
            continue;
        }

        for dir_idx in 0..6 {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

            let offset = DIRECTION_OFFSETS[dir_idx];
            let (n_chunk, n_local) = neighbor_chunk_local(entry.chunk, entry.local, offset);

            let n_current = sky_light_at(center_light, lights, center_pos, n_chunk, n_local);
            if n_current >= entry.level - 1 {
                continue;
            }

            let n_block = block_at(center_chunk, blocks, center_pos, n_chunk, n_local);
            let attenuation = if n_block.is_transparent_to_sky_light() {
                n_block.light_opacity().max(1)
            } else {
                15
            };

            let target = entry.level.saturating_sub(attenuation);
            if target > n_current
                && write_sky_light(
                    center_light,
                    lights,
                    center_pos,
                    n_chunk,
                    n_local,
                    target,
                    dirty_neighbors,
                )
            {
                let opposite_idx = dir_idx ^ 1;
                let next_dirs = ALL_DIRECTIONS_BITSET ^ (1 << opposite_idx);
                queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: target,
                    directions: next_dirs,
                });
            }
        }
    }
}

fn propagate_block_increase(
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    center_chunk: &Chunk,
    queue: &mut VecDeque<IncreaseEntry>,
    dirty_neighbors: &mut u32,
) {
    while let Some(entry) = queue.pop_front() {
        if entry.level <= 1 {
            continue;
        }

        for dir_idx in 0..6 {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

            let offset = DIRECTION_OFFSETS[dir_idx];
            let (n_chunk, n_local) = neighbor_chunk_local(entry.chunk, entry.local, offset);

            let n_current = block_light_at(center_light, lights, center_pos, n_chunk, n_local);
            if n_current >= entry.level - 1 {
                continue;
            }

            let n_block = block_at(center_chunk, blocks, center_pos, n_chunk, n_local);
            let attenuation = n_block.light_opacity().max(1);

            let target = entry.level.saturating_sub(attenuation);
            if target > n_current
                && write_block_light(
                    center_light,
                    lights,
                    center_pos,
                    n_chunk,
                    n_local,
                    target,
                    dirty_neighbors,
                )
            {
                let opposite_idx = dir_idx ^ 1;
                let next_dirs = ALL_DIRECTIONS_BITSET ^ (1 << opposite_idx);
                queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: target,
                    directions: next_dirs,
                });
            }
        }
    }
}

// ── Starlight decrease propagation ──────────────────────────────────────────
//
// Decrease is used when a block is placed (solidifying previously lit space).
// For each direction, checks if the neighbor's light exceeds what it SHOULD
// receive through the new opacity. If it does, sets it to 0 and enqueues for
// further decrease. Also detects clobbered light sources (emission) and
// re-enqueues them as INCREASE entries.
//
// After all decreases are processed, increase propagation runs to re-spread
// the re-discovered sources.

fn propagate_sky_decrease(
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    center_chunk: &Chunk,
    queue: &mut VecDeque<DecreaseEntry>,
    increase_queue: &mut VecDeque<IncreaseEntry>,
    dirty_neighbors: &mut u32,
) {
    while let Some(entry) = queue.pop_front() {
        for dir_idx in 0..6 {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

            let offset = DIRECTION_OFFSETS[dir_idx];
            let (n_chunk, n_local) = neighbor_chunk_local(entry.chunk, entry.local, offset);

            let n_current = sky_light_at(center_light, lights, center_pos, n_chunk, n_local);
            if n_current == 0 {
                continue;
            }

            let n_block = block_at(center_chunk, blocks, center_pos, n_chunk, n_local);
            let attenuation = if n_block.is_transparent_to_sky_light() {
                n_block.light_opacity().max(1)
            } else {
                15
            };

            let target = entry.level.saturating_sub(attenuation);

            if n_current > target {
                let opposite_idx = dir_idx ^ 1;
                let mut exclude = 1u8 << opposite_idx;
                if n_chunk.x != 0 {
                    exclude |= 1 << if n_chunk.x < 0 { 0 } else { 1 };
                }
                if n_chunk.z != 0 {
                    exclude |= 1 << if n_chunk.z < 0 { 2 } else { 3 };
                }
                if n_chunk.y != 0 {
                    exclude |= 1 << if n_chunk.y < 0 { 4 } else { 5 };
                }
                let next_dirs = ALL_DIRECTIONS_BITSET ^ exclude;
                increase_queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: n_current,
                    directions: next_dirs,
                });
            }

            if write_sky_light(
                center_light,
                lights,
                center_pos,
                n_chunk,
                n_local,
                0,
                dirty_neighbors,
            ) && target > 0
            {
                let opposite_idx = dir_idx ^ 1;
                let next_dirs = ALL_DIRECTIONS_BITSET ^ (1 << opposite_idx);
                queue.push_back(DecreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: target,
                    directions: next_dirs,
                });
            }
        }
    }

    propagate_sky_increase(
        center_light,
        blocks,
        lights,
        center_pos,
        center_chunk,
        increase_queue,
        dirty_neighbors,
    );
}

fn propagate_block_decrease(
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    center_chunk: &Chunk,
    queue: &mut VecDeque<DecreaseEntry>,
    increase_queue: &mut VecDeque<IncreaseEntry>,
    dirty_neighbors: &mut u32,
    clobbered_increases: bool,
) {
    while let Some(entry) = queue.pop_front() {
        for dir_idx in 0..6 {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

            let offset = DIRECTION_OFFSETS[dir_idx];
            let (n_chunk, n_local) = neighbor_chunk_local(entry.chunk, entry.local, offset);

            let n_current = block_light_at(center_light, lights, center_pos, n_chunk, n_local);
            if n_current == 0 {
                continue;
            }

            let n_block = block_at(center_chunk, blocks, center_pos, n_chunk, n_local);
            let attenuation = n_block.light_opacity().max(1);

            let target = entry.level.saturating_sub(attenuation);

            if clobbered_increases && n_current > target {
                let opposite_idx = dir_idx ^ 1;
                let mut exclude = 1u8 << opposite_idx;
                if n_chunk.x != 0 {
                    exclude |= 1 << if n_chunk.x < 0 { 0 } else { 1 };
                }
                if n_chunk.z != 0 {
                    exclude |= 1 << if n_chunk.z < 0 { 2 } else { 3 };
                }
                if n_chunk.y != 0 {
                    exclude |= 1 << if n_chunk.y < 0 { 4 } else { 5 };
                }
                let next_dirs = ALL_DIRECTIONS_BITSET ^ exclude;
                increase_queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: n_current,
                    directions: next_dirs,
                });
            }

            let emitted = n_block.light_emission();
            if emitted > 0 {
                write_block_light(
                    center_light,
                    lights,
                    center_pos,
                    n_chunk,
                    n_local,
                    emitted,
                    dirty_neighbors,
                );
                increase_queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: emitted,
                    directions: ALL_DIRECTIONS_BITSET,
                });
            }

            if write_block_light(
                center_light,
                lights,
                center_pos,
                n_chunk,
                n_local,
                0,
                dirty_neighbors,
            ) && target > 0
            {
                let opposite_idx = dir_idx ^ 1;
                let next_dirs = ALL_DIRECTIONS_BITSET ^ (1 << opposite_idx);
                queue.push_back(DecreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: target,
                    directions: next_dirs,
                });
            }
        }
    }

    propagate_block_increase(
        center_light,
        blocks,
        lights,
        center_pos,
        center_chunk,
        increase_queue,
        dirty_neighbors,
    );
}

// ── Sky light computation ───────────────────────────────────────────────────

fn has_darker_neighbor(
    center: &ChunkLight,
    lights: &HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    local: UVec3,
    level: u8,
) -> bool {
    for dir_idx in 0..6 {
        let offset = DIRECTION_OFFSETS[dir_idx];
        let (n_chunk, n_local) = neighbor_chunk_local(center_pos, local, offset);
        if sky_light_at(center, lights, center_pos, n_chunk, n_local) < level {
            return true;
        }
    }
    false
}

pub fn compute_sky_light(
    center_chunk: &Chunk,
    center_light: &mut ChunkLight,
    heightmap: &mut ChunkHeightmap,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    dirty_neighbors: &mut u32,
    column_y: u32,
    skip_heightmap: bool,
) {
    let center_pos = IVec3::ZERO;

    center_light.reset_all_sky_light();

    // Vertical top-down pass: propagate sky light from y=15 downward through
    // transparent blocks in each (x,z) column. Stop at first opaque block.
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let mut current_sky = SKY_LIGHT_MAX;
            let mut highest = 0u8;

            for y in (0..CHUNK_SIZE).rev() {
                let block = center_chunk.blocks[x][z][y];
                if !block.is_transparent_to_sky_light() {
                    // Safety: heightmap values stored as u8; column_y + y must
                    // fit in u8. Default 5 sub-chunks → max column_y=64 → max y=79.
                    // If height_chunks ever exceeds 16 (256 world height), the
                    // cast will silently truncate.
                    highest = (column_y as usize + y) as u8;
                    break;
                }

                let attenuation = block.light_opacity();
                current_sky = current_sky.saturating_sub(attenuation);
                if current_sky > 0 {
                    center_light.set_sky_light(uvec3(x as u32, y as u32, z as u32), current_sky);
                }
            }

            if !skip_heightmap {
                heightmap.heights[x][z] = highest;
            }
        }
    }

    // Seed increase queue only with blocks that can actually propagate:
    // - level > 1 (level 1 reaches nothing)
    // - for max-level blocks (15): only enqueue if a neighbor is darker
    //   (avoids enqueueing interior sky blocks that have nothing to fill)
    // - for attenuated blocks (<15): always enqueue (they're the frontier)
    let mut increase_queue = VecDeque::new();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                let pos = uvec3(x as u32, y as u32, z as u32);
                let sl = center_light.sky_light(pos);
                if sl <= 1 {
                    continue;
                }
                if sl == SKY_LIGHT_MAX
                    && !has_darker_neighbor(center_light, lights, center_pos, pos, sl)
                {
                    continue;
                }
                increase_queue.push_back(IncreaseEntry {
                    chunk: center_pos,
                    local: pos,
                    level: sl,
                    directions: ALL_DIRECTIONS_BITSET,
                });
            }
        }
    }

    propagate_sky_increase(
        center_light,
        blocks,
        lights,
        center_pos,
        center_chunk,
        &mut increase_queue,
        dirty_neighbors,
    );
}

// ── Block light computation ─────────────────────────────────────────────────

pub fn compute_block_light(
    center_chunk: &Chunk,
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    dirty_neighbors: &mut u32,
) {
    let center_pos = IVec3::ZERO;

    center_light.reset_all_block_light();

    // Seed increase queue with emitter blocks.
    let mut increase_queue = VecDeque::new();
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                let pos = uvec3(x as u32, y as u32, z as u32);
                let emission = center_chunk.get_block(pos).light_emission();
                if emission > 0 {
                    center_light.set_block_light(pos, emission);
                    increase_queue.push_back(IncreaseEntry {
                        chunk: center_pos,
                        local: pos,
                        level: emission,
                        directions: ALL_DIRECTIONS_BITSET,
                    });
                }
            }
        }
    }

    propagate_block_increase(
        center_light,
        blocks,
        lights,
        center_pos,
        center_chunk,
        &mut increase_queue,
        dirty_neighbors,
    );
}

/// Pull block light from illuminated neighbor face blocks into the center.
/// For freshly loaded chunks, neighbors may have emitters whose light should
/// propagate through the boundary into the center. Call after `compute_block_light`.
pub fn pull_neighbor_block_light(
    center_chunk: &Chunk,
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    dirty_neighbors: &mut u32,
) {
    let center_pos = IVec3::ZERO;

    let mut increase_queue = VecDeque::new();
    for &offset in &DIRECTION_OFFSETS {
        if !lights.contains_key(&offset) || !blocks.contains_key(&offset) {
            continue;
        }
        let neighbor_light = &lights[&offset];
        for a in 0..CHUNK_SIZE {
            for b in 0..CHUNK_SIZE {
                let Some((center_local, neighbor_local)) = face_local_pair(offset, a, b) else {
                    continue;
                };

                let n_level = neighbor_light.block_light(neighbor_local);
                if n_level <= 1 {
                    continue;
                }

                let center_block = center_chunk.get_block(center_local);
                let attenuation = center_block.light_opacity().max(1);
                let target = n_level.saturating_sub(attenuation);

                let current = center_light.block_light(center_local);
                if target > current {
                    center_light.set_block_light(center_local, target);
                    increase_queue.push_back(IncreaseEntry {
                        chunk: center_pos,
                        local: center_local,
                        level: target,
                        directions: ALL_DIRECTIONS_BITSET,
                    });
                }
            }
        }
    }

    if !increase_queue.is_empty() {
        propagate_block_increase(
            center_light,
            blocks,
            lights,
            center_pos,
            center_chunk,
            &mut increase_queue,
            dirty_neighbors,
        );
    }
}

/// Clear stale block light from neighbors after emitters are removed from the
/// center chunk. Compares each center face block's light against the adjacent
/// neighbor face block; if the neighbor light exceeds what the center could
/// have provided, propagates a decrease outward.
///
/// The center light is saved before the decrease cycle and restored afterward
/// (it was already computed correctly by `compute_block_light`).
pub fn clear_stale_neighbor_block_light(
    center_chunk: &Chunk,
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    dirty_neighbors: &mut u32,
) {
    let center_pos = IVec3::ZERO;

    let pre_decrease = center_light.clone();
    let mut block_decrease: VecDeque<DecreaseEntry> = VecDeque::new();
    let mut block_increase: VecDeque<IncreaseEntry> = VecDeque::new();
    for &offset in &DIRECTION_OFFSETS {
        if !lights.contains_key(&offset) {
            continue;
        }
        let neighbor_light = &lights[&offset];
        for a in 0..CHUNK_SIZE {
            for b in 0..CHUNK_SIZE {
                let Some((local, _)) = face_local_pair(offset, a, b) else {
                    continue;
                };
                let center_level = center_light.block_light(local);
                let (_, n_local) = neighbor_chunk_local(center_pos, local, offset);
                let n_current = neighbor_light.block_light(n_local);
                if n_current > center_level.saturating_sub(1) {
                    block_decrease.push_back(DecreaseEntry {
                        chunk: center_pos,
                        local,
                        level: n_current,
                        directions: ALL_DIRECTIONS_BITSET,
                    });
                }
            }
        }
    }
    if !block_decrease.is_empty() {
        propagate_block_decrease(
            center_light,
            blocks,
            lights,
            center_pos,
            center_chunk,
            &mut block_decrease,
            &mut block_increase,
            dirty_neighbors,
            false,
        );
    }
    *center_light = pre_decrease;
}

// ── Full light rebuild ─────────────────────────────────────────────────────-

pub fn compute_light(
    center_chunk: &Chunk,
    center_light: &mut ChunkLight,
    heightmap: &mut ChunkHeightmap,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    dirty_neighbors: &mut u32,
    _rendered: u16,
    column_y: u32,
    skip_heightmap: bool,
) {
    compute_sky_light(
        center_chunk,
        center_light,
        heightmap,
        blocks,
        lights,
        dirty_neighbors,
        column_y,
        skip_heightmap,
    );
    compute_block_light(center_chunk, center_light, blocks, lights, dirty_neighbors);
    pull_neighbor_block_light(center_chunk, center_light, blocks, lights, dirty_neighbors);
    clear_stale_neighbor_block_light(center_chunk, center_light, blocks, lights, dirty_neighbors);
}

// ── Region light rebuild ────────────────────────────────────────────────────

fn region_block_at(chunks: &HashMap<IVec3, &Chunk>, chunk_pos: IVec3, local: UVec3) -> BlockType {
    chunks
        .get(&chunk_pos)
        .map(|chunk| chunk.get_block(local))
        .unwrap_or(BlockType::Air)
}

fn region_sky_light_at(lights: &HashMap<IVec3, ChunkLight>, chunk_pos: IVec3, local: UVec3) -> u8 {
    lights
        .get(&chunk_pos)
        .map(|light| light.sky_light(local))
        .unwrap_or(0)
}

fn region_block_light_at(
    lights: &HashMap<IVec3, ChunkLight>,
    chunk_pos: IVec3,
    local: UVec3,
) -> u8 {
    lights
        .get(&chunk_pos)
        .map(|light| light.block_light(local))
        .unwrap_or(0)
}

fn write_region_sky_light(
    lights: &mut HashMap<IVec3, ChunkLight>,
    targets: &HashSet<IVec3>,
    chunk_pos: IVec3,
    local: UVec3,
    value: u8,
) -> bool {
    if !targets.contains(&chunk_pos) {
        return false;
    }

    let Some(light) = lights.get_mut(&chunk_pos) else {
        return false;
    };
    light.set_sky_light(local, value);
    true
}

fn write_region_block_light(
    lights: &mut HashMap<IVec3, ChunkLight>,
    targets: &HashSet<IVec3>,
    chunk_pos: IVec3,
    local: UVec3,
    value: u8,
) -> bool {
    if !targets.contains(&chunk_pos) {
        return false;
    }

    let Some(light) = lights.get_mut(&chunk_pos) else {
        return false;
    };
    light.set_block_light(local, value);
    true
}

fn propagate_region_sky_increase(
    chunks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    targets: &HashSet<IVec3>,
    queue: &mut VecDeque<IncreaseEntry>,
) {
    while let Some(entry) = queue.pop_front() {
        if entry.level <= 1 {
            continue;
        }

        for dir_idx in 0..6 {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

            let offset = DIRECTION_OFFSETS[dir_idx];
            let (n_chunk, n_local) = neighbor_chunk_local(entry.chunk, entry.local, offset);
            if !targets.contains(&n_chunk) {
                continue;
            }

            let n_current = region_sky_light_at(lights, n_chunk, n_local);
            if n_current >= entry.level - 1 {
                continue;
            }

            let n_block = region_block_at(chunks, n_chunk, n_local);
            let attenuation = if n_block.is_transparent_to_sky_light() {
                n_block.light_opacity().max(1)
            } else {
                15
            };
            let target = entry.level.saturating_sub(attenuation);
            if target > n_current
                && write_region_sky_light(lights, targets, n_chunk, n_local, target)
            {
                let opposite_idx = dir_idx ^ 1;
                queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: target,
                    directions: ALL_DIRECTIONS_BITSET ^ (1 << opposite_idx),
                });
            }
        }
    }
}

fn propagate_region_block_increase(
    chunks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    targets: &HashSet<IVec3>,
    queue: &mut VecDeque<IncreaseEntry>,
) {
    while let Some(entry) = queue.pop_front() {
        if entry.level <= 1 {
            continue;
        }

        for dir_idx in 0..6 {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

            let offset = DIRECTION_OFFSETS[dir_idx];
            let (n_chunk, n_local) = neighbor_chunk_local(entry.chunk, entry.local, offset);
            if !targets.contains(&n_chunk) {
                continue;
            }

            let n_current = region_block_light_at(lights, n_chunk, n_local);
            if n_current >= entry.level - 1 {
                continue;
            }

            let n_block = region_block_at(chunks, n_chunk, n_local);
            let target = entry.level.saturating_sub(n_block.light_opacity().max(1));
            if target > n_current
                && write_region_block_light(lights, targets, n_chunk, n_local, target)
            {
                let opposite_idx = dir_idx ^ 1;
                queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: target,
                    directions: ALL_DIRECTIONS_BITSET ^ (1 << opposite_idx),
                });
            }
        }
    }
}

fn seed_region_sky_sources(
    chunks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    heightmaps: &mut HashMap<IVec3, ChunkHeightmap>,
    targets: &HashSet<IVec3>,
    height_chunks: i32,
    queue: &mut VecDeque<IncreaseEntry>,
) {
    let columns = targets
        .iter()
        .map(|pos| ivec2(pos.x, pos.z))
        .collect::<HashSet<_>>();

    for column in columns {
        let top_chunk_loaded = chunks.contains_key(&ivec3(column.x, height_chunks - 1, column.y));
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let mut current_sky = if top_chunk_loaded { SKY_LIGHT_MAX } else { 0 };
                let mut highest = 0u8;
                let mut found_highest = false;

                for chunk_y in (0..height_chunks).rev() {
                    let chunk_pos = ivec3(column.x, chunk_y, column.y);
                    let Some(chunk) = chunks.get(&chunk_pos) else {
                        current_sky = 0;
                        continue;
                    };
                    let is_target = targets.contains(&chunk_pos);

                    for y in (0..CHUNK_SIZE).rev() {
                        let block = chunk.blocks[x][z][y];
                        if !block.is_transparent_to_sky_light() {
                            if !found_highest {
                                highest = (chunk_y * CHUNK_ISIZE + y as i32) as u8;
                                found_highest = true;
                            }
                            current_sky = 0;
                            continue;
                        }

                        current_sky = current_sky.saturating_sub(block.light_opacity());
                        if current_sky == 0 || !is_target {
                            continue;
                        }

                        let local = uvec3(x as u32, y as u32, z as u32);
                        write_region_sky_light(lights, targets, chunk_pos, local, current_sky);
                    }
                }

                for chunk_y in 0..height_chunks {
                    let chunk_pos = ivec3(column.x, chunk_y, column.y);
                    if !targets.contains(&chunk_pos) {
                        continue;
                    }
                    if let Some(heightmap) = heightmaps.get_mut(&chunk_pos) {
                        heightmap.heights[x][z] = highest;
                    }
                }
            }
        }
    }

    for &chunk_pos in targets {
        let Some(chunk) = chunks.get(&chunk_pos) else {
            continue;
        };

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let local = uvec3(x as u32, y as u32, z as u32);
                    let current = region_sky_light_at(lights, chunk_pos, local);
                    if current >= SKY_LIGHT_MAX - 1 {
                        continue;
                    }

                    let block = chunk.get_block(local);
                    if !block.is_transparent_to_sky_light() {
                        continue;
                    }

                    let attenuation = block.light_opacity().max(1);
                    let mut best = current;
                    for &offset in &DIRECTION_OFFSETS {
                        let (n_chunk, n_local) = neighbor_chunk_local(chunk_pos, local, offset);
                        if !targets.contains(&n_chunk) {
                            continue;
                        }

                        let target = region_sky_light_at(lights, n_chunk, n_local)
                            .saturating_sub(attenuation);
                        best = best.max(target);
                    }

                    if best > current
                        && write_region_sky_light(lights, targets, chunk_pos, local, best)
                    {
                        queue.push_back(IncreaseEntry {
                            chunk: chunk_pos,
                            local,
                            level: best,
                            directions: ALL_DIRECTIONS_BITSET,
                        });
                    }
                }
            }
        }
    }
}

fn seed_region_block_sources(
    chunks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    targets: &HashSet<IVec3>,
    queue: &mut VecDeque<IncreaseEntry>,
) {
    for &chunk_pos in targets {
        let Some(chunk) = chunks.get(&chunk_pos) else {
            continue;
        };

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let local = uvec3(x as u32, y as u32, z as u32);
                    let emission = chunk.get_block(local).light_emission();
                    if emission == 0 {
                        continue;
                    }

                    if write_region_block_light(lights, targets, chunk_pos, local, emission) {
                        queue.push_back(IncreaseEntry {
                            chunk: chunk_pos,
                            local,
                            level: emission,
                            directions: ALL_DIRECTIONS_BITSET,
                        });
                    }
                }
            }
        }
    }
}

fn seed_region_boundary_light(
    chunks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    targets: &HashSet<IVec3>,
    sky_queue: &mut VecDeque<IncreaseEntry>,
    block_queue: &mut VecDeque<IncreaseEntry>,
) {
    for &chunk_pos in targets {
        for &offset in &DIRECTION_OFFSETS {
            let neighbor_pos = chunk_pos + offset;
            if targets.contains(&neighbor_pos) {
                continue;
            }
            for a in 0..CHUNK_SIZE {
                for b in 0..CHUNK_SIZE {
                    let Some((local, neighbor_local)) = face_local_pair(offset, a, b) else {
                        continue;
                    };
                    let Some((sky_level, block_level)) = lights.get(&neighbor_pos).map(|light| {
                        (
                            light.sky_light(neighbor_local),
                            light.block_light(neighbor_local),
                        )
                    }) else {
                        continue;
                    };
                    let block = region_block_at(chunks, chunk_pos, local);

                    if sky_level > 0 {
                        let attenuation = if block.is_transparent_to_sky_light() {
                            if offset == IVec3::Y {
                                block.light_opacity()
                            } else {
                                block.light_opacity().max(1)
                            }
                        } else {
                            15
                        };
                        let target = sky_level.saturating_sub(attenuation);
                        if target > region_sky_light_at(lights, chunk_pos, local)
                            && write_region_sky_light(lights, targets, chunk_pos, local, target)
                            && target > 1
                        {
                            sky_queue.push_back(IncreaseEntry {
                                chunk: chunk_pos,
                                local,
                                level: target,
                                directions: ALL_DIRECTIONS_BITSET,
                            });
                        }
                    }

                    if block_level > 0 {
                        let target = block_level.saturating_sub(block.light_opacity().max(1));
                        if target > region_block_light_at(lights, chunk_pos, local)
                            && write_region_block_light(lights, targets, chunk_pos, local, target)
                            && target > 1
                        {
                            block_queue.push_back(IncreaseEntry {
                                chunk: chunk_pos,
                                local,
                                level: target,
                                directions: ALL_DIRECTIONS_BITSET,
                            });
                        }
                    }
                }
            }
        }
    }
}

pub fn compute_light_region(
    chunks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    heightmaps: &mut HashMap<IVec3, ChunkHeightmap>,
    targets: &HashSet<IVec3>,
    height_chunks: i32,
) {
    if height_chunks <= 0 || targets.is_empty() {
        return;
    }

    for &pos in targets {
        if chunks.contains_key(&pos) {
            lights.insert(pos, ChunkLight::default());
            heightmaps.entry(pos).or_default();
        }
    }

    let mut sky_queue = VecDeque::new();
    seed_region_sky_sources(
        chunks,
        lights,
        heightmaps,
        targets,
        height_chunks,
        &mut sky_queue,
    );

    let mut block_queue = VecDeque::new();
    seed_region_block_sources(chunks, lights, targets, &mut block_queue);
    seed_region_boundary_light(chunks, lights, targets, &mut sky_queue, &mut block_queue);

    propagate_region_sky_increase(chunks, lights, targets, &mut sky_queue);
    propagate_region_block_increase(chunks, lights, targets, &mut block_queue);
}

// ── Incremental block-change light updates (Starlight decrease+increase) ────

/// Recalculate light after a block at `world_pos` was placed (solidified).
/// Runs the Starlight decrease algorithm: zeroes blocks that can no longer
/// receive the same light through the new opacity, then re-propagates any
/// clobbered sources.
pub fn light_on_place_sky(
    center_chunk: &Chunk,
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    world_pos: IVec3,
    dirty_neighbors: &mut u32,
) {
    let center_pos = IVec3::ZERO;
    let (chunk, local) = world_to_chunk_local(world_pos);

    let old_level = sky_light_at(center_light, lights, center_pos, chunk, local);

    let placed_block = block_at(center_chunk, blocks, center_pos, chunk, local);
    let attenuation = if placed_block.is_transparent_to_sky_light() {
        placed_block.light_opacity().max(1)
    } else {
        15
    };

    let target = old_level.saturating_sub(attenuation);

    let mut decrease_queue = VecDeque::new();
    let mut increase_queue = VecDeque::new();

    decrease_queue.push_back(DecreaseEntry {
        chunk,
        local,
        level: target,
        directions: ALL_DIRECTIONS_BITSET,
    });

    propagate_sky_decrease(
        center_light,
        blocks,
        lights,
        center_pos,
        center_chunk,
        &mut decrease_queue,
        &mut increase_queue,
        dirty_neighbors,
    );
}

/// Recalculate light after a block at `world_pos` was placed (solidified).
pub fn light_on_place_block(
    center_chunk: &Chunk,
    center_light: &mut ChunkLight,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, ChunkLight>,
    world_pos: IVec3,
    dirty_neighbors: &mut u32,
) {
    let center_pos = IVec3::ZERO;
    let (chunk, local) = world_to_chunk_local(world_pos);

    let old_level = block_light_at(center_light, lights, center_pos, chunk, local);
    let placed_block = block_at(center_chunk, blocks, center_pos, chunk, local);
    let attenuation = placed_block.light_opacity().max(1);
    let target = old_level.saturating_sub(attenuation);

    let mut decrease_queue = VecDeque::new();
    let mut increase_queue = VecDeque::new();

    decrease_queue.push_back(DecreaseEntry {
        chunk,
        local,
        level: target,
        directions: ALL_DIRECTIONS_BITSET,
    });

    propagate_block_decrease(
        center_light,
        blocks,
        lights,
        center_pos,
        center_chunk,
        &mut decrease_queue,
        &mut increase_queue,
        dirty_neighbors,
        false,
    );
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chunk_with_blocks<F>(mut fill: F) -> Chunk
    where
        F: FnMut(u32, u32, u32) -> BlockType,
    {
        let mut chunk = Chunk::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    chunk.blocks[x][z][y] = fill(x as u32, y as u32, z as u32);
                }
            }
        }
        chunk
    }

    #[test]
    fn sky_light_vertical_pass_above_surface_is_full() {
        let chunk = test_chunk_with_blocks(|_, y, _| {
            if y < 10 {
                BlockType::Stone
            } else {
                BlockType::Air
            }
        });
        let mut light = ChunkLight::default();
        let mut heightmap = ChunkHeightmap::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();

        let mut dirty = 0;
        compute_sky_light(
            &chunk,
            &mut light,
            &mut heightmap,
            &blocks,
            &mut lights,
            &mut dirty,
            0,
            false,
        );

        assert_eq!(heightmap.heights[8][8], 9);
        for y in 10..16 {
            assert_eq!(
                light.sky_light(uvec3(8, y, 8)),
                SKY_LIGHT_MAX,
                "sky light above surface at y={y}"
            );
        }
    }

    #[test]
    fn sky_light_vertical_pass_attenuates_through_transparent() {
        let chunk = test_chunk_with_blocks(|x, y, z| {
            let _ = (x, z);
            if y < 10 {
                BlockType::Stone
            } else if y < 13 {
                BlockType::OakLeaves
            } else {
                BlockType::Air
            }
        });
        let mut light = ChunkLight::default();
        let mut heightmap = ChunkHeightmap::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();

        let mut dirty = 0;
        compute_sky_light(
            &chunk,
            &mut light,
            &mut heightmap,
            &blocks,
            &mut lights,
            &mut dirty,
            0,
            false,
        );

        assert_eq!(heightmap.heights[0][0], 9);
        assert_eq!(light.sky_light(uvec3(0, 15, 0)), SKY_LIGHT_MAX);
        assert_eq!(light.sky_light(uvec3(0, 14, 0)), SKY_LIGHT_MAX);
        assert_eq!(light.sky_light(uvec3(0, 12, 0)), SKY_LIGHT_MAX - 1);
        assert_eq!(light.sky_light(uvec3(0, 11, 0)), SKY_LIGHT_MAX - 2);
        assert_eq!(light.sky_light(uvec3(0, 9, 0)), 0);
    }

    #[test]
    fn sky_light_vertical_pass_fully_opaque_stops_light() {
        let chunk = test_chunk_with_blocks(|_x, y, _z| {
            if y < 10 {
                BlockType::Stone
            } else {
                BlockType::Air
            }
        });
        let mut light = ChunkLight::default();
        let mut heightmap = ChunkHeightmap::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();

        let mut dirty = 0;
        compute_sky_light(
            &chunk,
            &mut light,
            &mut heightmap,
            &blocks,
            &mut lights,
            &mut dirty,
            0,
            false,
        );

        assert_eq!(heightmap.heights[0][0], 9);
        for y in 10..16 {
            assert_eq!(
                light.sky_light(uvec3(0, y as u32, 0)),
                15,
                "sky_light at y={y} should be 15"
            );
        }
        for y in 0..10 {
            let sl = light.sky_light(uvec3(0, y as u32, 0));
            assert_eq!(
                sl, 0,
                "sky_light at y={y} should be 0, but got {sl}. Block={:?}",
                chunk.blocks[0][0][y as usize]
            );
        }
    }

    #[test]
    fn sky_light_horizontal_bfs_into_cave() {
        let chunk = test_chunk_with_blocks(|x, y, z| {
            if x == 0 && z == 8 {
                BlockType::Air
            } else if x == 1 && z == 8 && (6..=10).contains(&y) {
                BlockType::Air
            } else {
                BlockType::Stone
            }
        });

        let mut light = ChunkLight::default();
        let mut heightmap = ChunkHeightmap::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();

        let mut dirty = 0;
        compute_sky_light(
            &chunk,
            &mut light,
            &mut heightmap,
            &blocks,
            &mut lights,
            &mut dirty,
            0,
            false,
        );

        assert_eq!(light.sky_light(uvec3(0, 15, 8)), SKY_LIGHT_MAX);
        assert!(light.sky_light(uvec3(1, 8, 8)) > 0);
        assert!(light.sky_light(uvec3(1, 8, 8)) < SKY_LIGHT_MAX);
    }

    #[test]
    fn block_light_bfs_emitter_propagates() {
        let mut chunk = Chunk::default();
        chunk.blocks[8][8][8] = BlockType::Glowstone;

        let mut light = ChunkLight::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();

        let mut dirty = 0;
        compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);

        assert_eq!(light.block_light(uvec3(8, 8, 8)), 15);
        assert_eq!(light.block_light(uvec3(7, 8, 8)), 14);
        assert_eq!(light.block_light(uvec3(8, 9, 8)), 14);
        assert_eq!(light.block_light(uvec3(6, 8, 8)), 13);
    }

    #[test]
    fn block_light_bfs_stopped_by_opaque() {
        let mut chunk = Chunk::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    chunk.blocks[x][z][y] = BlockType::Stone;
                }
            }
        }
        chunk.blocks[8][8][8] = BlockType::Glowstone;
        chunk.blocks[7][8][8] = BlockType::Air;

        let mut light = ChunkLight::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();

        let mut dirty = 0;
        compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);

        assert_eq!(light.block_light(uvec3(8, 8, 8)), 15);
        assert_eq!(light.block_light(uvec3(7, 8, 8)), 14);
        assert_eq!(light.block_light(uvec3(6, 8, 8)), 0);
    }

    #[test]
    fn block_light_bfs_through_transparent() {
        let mut chunk = Chunk::default();
        chunk.blocks[8][8][8] = BlockType::Glowstone;
        chunk.blocks[7][8][8] = BlockType::OakLeaves;

        let mut light = ChunkLight::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();

        let mut dirty = 0;
        compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);

        assert_eq!(light.block_light(uvec3(8, 8, 8)), 15);
        assert_eq!(light.block_light(uvec3(7, 8, 8)), 14);
        assert_eq!(light.block_light(uvec3(6, 8, 8)), 13);
    }

    #[test]
    fn cross_chunk_sky_light_propagates_upward() {
        let lower_chunk = Chunk::default();
        let mut lower_light = ChunkLight::default();
        let mut upper_chunk = Chunk::default();
        let mut upper_light = ChunkLight::default();
        let mut heightmap = ChunkHeightmap::default();

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                upper_chunk.blocks[x][z][15] = BlockType::Air;
            }
        }

        lower_light.set_sky_light(uvec3(8, 15, 8), SKY_LIGHT_MAX);

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(0, -1, 0), &lower_chunk)]);
        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(0, -1, 0), lower_light)]);
        let mut dirty = 0;
        compute_sky_light(
            &upper_chunk,
            &mut upper_light,
            &mut heightmap,
            &blocks,
            &mut lights,
            &mut dirty,
            0,
            false,
        );

        assert!(upper_light.sky_light(uvec3(8, 0, 8)) > 0);
    }

    #[test]
    fn cross_chunk_block_light_propagates_between_chunks() {
        let left_chunk = Chunk::default();
        let left_light = ChunkLight::default();
        let mut right_chunk = Chunk::default();
        let mut right_light = ChunkLight::default();

        right_chunk.blocks[0][8][8] = BlockType::Glowstone;

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), left_light.clone())]);
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        let modified_left = lights.remove(&ivec3(-1, 0, 0)).unwrap();

        assert_eq!(right_light.block_light(uvec3(0, 8, 8)), 15);
        assert_eq!(right_light.block_light(uvec3(1, 8, 8)), 14);
        assert_eq!(modified_left.block_light(uvec3(15, 8, 8)), 14);
    }

    #[test]
    fn block_light_decrease_when_emitter_removed() {
        let left_chunk = Chunk::default();
        let mut right_chunk = Chunk::default();
        let mut right_light = ChunkLight::default();

        right_chunk.blocks[0][8][8] = BlockType::Glowstone;

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

        // First: propagate light from Glowstone into neighbor.
        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), ChunkLight::default())]);
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        let left_after_emit = lights.remove(&ivec3(-1, 0, 0)).unwrap();
        assert_eq!(left_after_emit.block_light(uvec3(15, 8, 8)), 14);

        // Second: remove Glowstone and recompute.
        right_chunk.blocks[0][8][8] = BlockType::Air;
        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), left_after_emit)]);
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        clear_stale_neighbor_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        let left_after_remove = lights.remove(&ivec3(-1, 0, 0)).unwrap();

        assert_eq!(
            right_light.block_light(uvec3(0, 8, 8)),
            0,
            "emitter position should be 0"
        );
        assert_eq!(
            right_light.block_light(uvec3(1, 8, 8)),
            0,
            "no emitter means no center propagation"
        );
        assert_eq!(
            left_after_remove.block_light(uvec3(15, 8, 8)),
            0,
            "neighbor light must clear when emitter is removed"
        );
    }

    #[test]
    fn light_packed_roundtrip() {
        let mut light = ChunkLight::default();
        let pos = uvec3(7, 5, 3);
        light.set_sky_light(pos, 13);
        light.set_block_light(pos, 9);

        assert_eq!(light.sky_light(pos), 13);
        assert_eq!(light.block_light(pos), 9);
        let packed = light.packed_light(pos);
        assert_eq!((packed >> 4) & 0x0F, 13);
        assert_eq!(packed & 0x0F, 9);
    }

    #[test]
    fn padded_light_data_packs_four_cells_per_word() {
        let center_pos = IVec3::ZERO;
        let mut center = ChunkLight::default();
        center.set_sky_light(uvec3(0, 0, 0), 1);
        center.set_block_light(uvec3(0, 0, 0), 2);

        let mut right = ChunkLight::default();
        right.set_sky_light(uvec3(0, 0, 0), 10);
        right.set_block_light(uvec3(0, 0, 0), 11);

        let lights = HashMap::from([(center_pos, &center), (IVec3::X, &right)]);
        let data = ChunkLight::build_padded_light_data(center_pos, &lights);

        assert_eq!(data.len(), PADDED_LIGHT_WORDS);
        assert_eq!(
            unpack_padded_light(&data, padded_light_index(1, 1, 1)),
            0x12
        );
        assert_eq!(
            unpack_padded_light(&data, padded_light_index(17, 1, 1)),
            0xAB
        );
        assert_eq!(
            unpack_padded_light(&data, padded_light_index(0, 0, 0)),
            0xF0
        );
    }

    fn padded_light_index(x: usize, y: usize, z: usize) -> usize {
        x + z * PADDED_CHUNK_SIZE + y * PADDED_CHUNK_LAYER_SIZE
    }

    fn unpack_padded_light(data: &[u32], idx: usize) -> u8 {
        ((data[idx / 4] >> ((idx % 4) * 8)) & 0xFF) as u8
    }

    #[test]
    fn light_resets_clear_correctly() {
        let mut light = ChunkLight::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    light.set_sky_light(uvec3(x as u32, y as u32, z as u32), 15);
                    light.set_block_light(uvec3(x as u32, y as u32, z as u32), 15);
                }
            }
        }
        light.reset_all_sky_light();
        light.reset_all_block_light();

        assert_eq!(light.packed_light(uvec3(8, 8, 8)), 0);
    }

    #[test]
    fn heightmap_all_air_chunk_is_zero() {
        let chunk = Chunk::default();
        let mut light = ChunkLight::default();
        let mut heightmap = ChunkHeightmap::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();

        let mut dirty = 0;
        compute_sky_light(
            &chunk,
            &mut light,
            &mut heightmap,
            &blocks,
            &mut lights,
            &mut dirty,
            0,
            false,
        );

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                assert_eq!(heightmap.heights[x][z], 0);
            }
        }
    }

    fn empty_chunk() -> Chunk {
        Chunk::default()
    }

    fn target_set(positions: impl IntoIterator<Item = IVec3>) -> HashSet<IVec3> {
        positions.into_iter().collect()
    }

    #[test]
    fn region_sky_occlusion_spans_vertical_chunks() {
        let lower = empty_chunk();
        let mut upper = empty_chunk();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                upper.blocks[x][z][0] = BlockType::Stone;
            }
        }

        let chunks: HashMap<IVec3, &Chunk> =
            HashMap::from([(ivec3(0, 0, 0), &lower), (ivec3(0, 1, 0), &upper)]);
        let mut lights = HashMap::from([
            (ivec3(0, 0, 0), ChunkLight::default()),
            (ivec3(0, 1, 0), ChunkLight::default()),
        ]);
        let mut heightmaps = HashMap::from([
            (ivec3(0, 0, 0), ChunkHeightmap::default()),
            (ivec3(0, 1, 0), ChunkHeightmap::default()),
        ]);
        let targets = target_set([ivec3(0, 0, 0), ivec3(0, 1, 0)]);

        compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 2);

        assert_eq!(lights[&ivec3(0, 1, 0)].sky_light(uvec3(8, 1, 8)), 15);
        assert_eq!(lights[&ivec3(0, 1, 0)].sky_light(uvec3(8, 0, 8)), 0);
        assert_eq!(lights[&ivec3(0, 0, 0)].sky_light(uvec3(8, 15, 8)), 0);
        assert_eq!(heightmaps[&ivec3(0, 0, 0)].heights[8][8], 16);
        assert_eq!(heightmaps[&ivec3(0, 1, 0)].heights[8][8], 16);
    }

    #[test]
    fn region_sky_waits_for_missing_upper_chunk() {
        let lower = empty_chunk();
        let chunks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(0, 0, 0), &lower)]);
        let mut lights = HashMap::from([(ivec3(0, 0, 0), ChunkLight::default())]);
        let mut heightmaps = HashMap::from([(ivec3(0, 0, 0), ChunkHeightmap::default())]);
        let targets = target_set([ivec3(0, 0, 0)]);

        compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 2);

        assert_eq!(lights[&ivec3(0, 0, 0)].sky_light(uvec3(8, 15, 8)), 0);
    }

    #[test]
    fn region_all_air_chunk_clears_stale_block_light() {
        let chunk = empty_chunk();
        let pos = IVec3::ZERO;
        let chunks: HashMap<IVec3, &Chunk> = HashMap::from([(pos, &chunk)]);
        let mut stale = ChunkLight::default();
        stale.set_block_light(uvec3(8, 8, 8), 15);
        let mut lights = HashMap::from([(pos, stale)]);
        let mut heightmaps = HashMap::from([(pos, ChunkHeightmap::default())]);
        let targets = target_set([pos]);

        compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 1);

        assert_eq!(lights[&pos].block_light(uvec3(8, 8, 8)), 0);
        assert_eq!(lights[&pos].sky_light(uvec3(8, 8, 8)), 15);
    }

    #[test]
    fn region_block_light_crosses_y_boundary() {
        let lower = empty_chunk();
        let mut upper = empty_chunk();
        upper.blocks[8][8][0] = BlockType::Glowstone;
        let chunks: HashMap<IVec3, &Chunk> =
            HashMap::from([(ivec3(0, 0, 0), &lower), (ivec3(0, 1, 0), &upper)]);
        let mut lights = HashMap::from([
            (ivec3(0, 0, 0), ChunkLight::default()),
            (ivec3(0, 1, 0), ChunkLight::default()),
        ]);
        let mut heightmaps = HashMap::from([
            (ivec3(0, 0, 0), ChunkHeightmap::default()),
            (ivec3(0, 1, 0), ChunkHeightmap::default()),
        ]);
        let targets = target_set([ivec3(0, 0, 0), ivec3(0, 1, 0)]);

        compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 2);

        assert_eq!(lights[&ivec3(0, 1, 0)].block_light(uvec3(8, 0, 8)), 15);
        assert_eq!(lights[&ivec3(0, 0, 0)].block_light(uvec3(8, 15, 8)), 14);
    }

    #[test]
    fn region_block_light_crosses_z_boundary() {
        let center = empty_chunk();
        let mut back = empty_chunk();
        back.blocks[8][0][8] = BlockType::Glowstone;
        let chunks: HashMap<IVec3, &Chunk> =
            HashMap::from([(IVec3::ZERO, &center), (IVec3::Z, &back)]);
        let mut lights = HashMap::from([
            (IVec3::ZERO, ChunkLight::default()),
            (IVec3::Z, ChunkLight::default()),
        ]);
        let mut heightmaps = HashMap::from([
            (IVec3::ZERO, ChunkHeightmap::default()),
            (IVec3::Z, ChunkHeightmap::default()),
        ]);
        let targets = target_set([IVec3::ZERO, IVec3::Z]);

        compute_light_region(&chunks, &mut lights, &mut heightmaps, &targets, 1);

        assert_eq!(lights[&IVec3::Z].block_light(uvec3(8, 8, 0)), 15);
        assert_eq!(lights[&IVec3::ZERO].block_light(uvec3(8, 8, 15)), 14);
    }

    fn neighbor_with_glowstone(x: u32, y: u32, z: u32) -> (Chunk, ChunkLight) {
        let mut chunk = empty_chunk();
        chunk.blocks[x as usize][z as usize][y as usize] = BlockType::Glowstone;
        let mut light = ChunkLight::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();
        let mut dirty = 0;
        compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);
        (chunk, light)
    }

    // ── Pull-from-neighbor tests ──────────────────────────────────────────

    #[test]
    fn block_light_pulls_from_neighbor_emitter() {
        let left_chunk = empty_chunk();
        let mut left_light = ChunkLight::default();
        left_light.set_block_light(uvec3(15, 8, 8), 15);
        left_light.set_block_light(uvec3(14, 8, 8), 14);
        left_light.set_block_light(uvec3(15, 9, 8), 14);

        let right_chunk = empty_chunk();
        let mut right_light = ChunkLight::default();

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);
        let mut lights: HashMap<IVec3, ChunkLight> = HashMap::from([(ivec3(-1, 0, 0), left_light)]);
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        pull_neighbor_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );

        assert!(
            right_light.block_light(uvec3(0, 8, 8)) >= 14,
            "center must pull light from neighbor emitter; got {}",
            right_light.block_light(uvec3(0, 8, 8))
        );
        assert!(
            right_light.block_light(uvec3(1, 8, 8)) >= 13,
            "center propagation from pulled face light; got {}",
            right_light.block_light(uvec3(1, 8, 8))
        );
    }

    #[test]
    fn block_light_pulls_from_neighbor_emitter_across_corner() {
        let diag_chunk = empty_chunk();
        let mut diag_light = ChunkLight::default();
        diag_light.set_block_light(uvec3(15, 8, 0), 15);
        diag_light.set_block_light(uvec3(14, 8, 0), 14);
        let center_chunk = empty_chunk();
        let mut center_light = ChunkLight::default();

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &diag_chunk)]);
        let mut lights: HashMap<IVec3, ChunkLight> = HashMap::from([(ivec3(-1, 0, 0), diag_light)]);
        let mut dirty = 0;
        compute_block_light(
            &center_chunk,
            &mut center_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        pull_neighbor_block_light(
            &center_chunk,
            &mut center_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );

        assert!(
            center_light.block_light(uvec3(0, 8, 0)) >= 14,
            "corner neighbor light must pull into center; got {}",
            center_light.block_light(uvec3(0, 8, 0))
        );
    }

    #[test]
    fn empty_chunk_does_not_clear_neighbor_own_emitter_light() {
        let (left_chunk, left_light) = neighbor_with_glowstone(15, 8, 8);
        let right_chunk = empty_chunk();
        let mut right_light = ChunkLight::default();

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);
        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), left_light.clone())]);
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );

        let modified_left = lights.get(&ivec3(-1, 0, 0)).unwrap();
        assert_eq!(
            modified_left.block_light(uvec3(15, 8, 8)),
            15,
            "neighbor Glowstone must remain lit"
        );
        assert!(
            modified_left.block_light(uvec3(14, 8, 8)) > 0,
            "neighbor propagation from its own emitter must survive"
        );
    }

    #[test]
    fn empty_chunk_neighbor_pull_from_multiple_sides() {
        let left_chunk = empty_chunk();
        let mut left_light = ChunkLight::default();
        left_light.set_block_light(uvec3(15, 8, 8), 15);
        left_light.set_block_light(uvec3(14, 8, 8), 14);

        let right_chunk = empty_chunk();
        let mut right_light = ChunkLight::default();
        right_light.set_block_light(uvec3(0, 8, 8), 15);
        right_light.set_block_light(uvec3(1, 8, 8), 14);

        let center_chunk = empty_chunk();
        let mut center_light = ChunkLight::default();

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([
            (ivec3(-1, 0, 0), &left_chunk),
            (ivec3(1, 0, 0), &right_chunk),
        ]);
        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), left_light), (ivec3(1, 0, 0), right_light)]);
        let mut dirty = 0;
        compute_block_light(
            &center_chunk,
            &mut center_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        pull_neighbor_block_light(
            &center_chunk,
            &mut center_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );

        assert!(
            center_light.block_light(uvec3(0, 8, 8)) >= 14,
            "light from left neighbor face; got {}",
            center_light.block_light(uvec3(0, 8, 8))
        );
        assert!(
            center_light.block_light(uvec3(15, 8, 8)) >= 14,
            "light from right neighbor face; got {}",
            center_light.block_light(uvec3(15, 8, 8))
        );
        let mid = center_light.block_light(uvec3(7, 8, 8));
        assert!(
            mid > 0,
            "midpoint should receive converging light; got {}",
            mid
        );
    }

    // ── Multiple emitters ─────────────────────────────────────────────────

    #[test]
    fn multiple_emitters_propagate_independently() {
        let mut chunk = empty_chunk();
        chunk.blocks[4][4][8] = BlockType::Glowstone;
        chunk.blocks[12][12][8] = BlockType::Glowstone;

        let mut light = ChunkLight::default();
        let blocks = HashMap::new();
        let mut lights = HashMap::new();
        let mut dirty = 0;
        compute_block_light(&chunk, &mut light, &blocks, &mut lights, &mut dirty);

        assert_eq!(light.block_light(uvec3(4, 8, 4)), 15);
        assert_eq!(light.block_light(uvec3(12, 8, 12)), 15);
        assert!(
            light.block_light(uvec3(7, 8, 7)) > 0,
            "midpoint between emitters must receive light"
        );
    }

    #[test]
    fn multiple_emitters_on_faces_propagate_cross_chunk() {
        let left_chunk = empty_chunk();
        let mut right_chunk = empty_chunk();

        let left_light = ChunkLight::default();
        let mut right_light = ChunkLight::default();

        right_chunk.blocks[0][8][8] = BlockType::Glowstone;
        right_chunk.blocks[0][4][8] = BlockType::Glowstone;
        right_chunk.blocks[0][12][8] = BlockType::Glowstone;

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);
        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), left_light.clone())]);
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        let modified_left = lights.remove(&ivec3(-1, 0, 0)).unwrap();

        assert_eq!(modified_left.block_light(uvec3(15, 8, 8)), 14);
        assert_eq!(modified_left.block_light(uvec3(15, 8, 4)), 14);
        assert_eq!(modified_left.block_light(uvec3(15, 8, 12)), 14);
    }

    // ── Emitter removal with neighborhood ─────────────────────────────────

    #[test]
    fn removal_of_one_emitter_preserves_other_emitter_light() {
        let left_chunk = empty_chunk();
        let mut right_chunk = empty_chunk();
        right_chunk.blocks[0][8][8] = BlockType::Glowstone;
        right_chunk.blocks[0][8][10] = BlockType::Glowstone;

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), ChunkLight::default())]);
        let mut right_light = ChunkLight::default();
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );

        let left_after_both = lights.remove(&ivec3(-1, 0, 0)).unwrap();
        assert!(left_after_both.block_light(uvec3(15, 8, 8)) > 0);

        right_chunk.blocks[0][8][8] = BlockType::Air;
        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), left_after_both)]);
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        clear_stale_neighbor_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );

        let left_after_remove = lights.get(&ivec3(-1, 0, 0)).unwrap();
        assert_eq!(
            right_light.block_light(uvec3(0, 10, 8)),
            15,
            "remaining emitter must stay lit"
        );
        assert!(
            left_after_remove.block_light(uvec3(15, 10, 8)) > 0,
            "neighbor light from remaining emitter must survive; got 0"
        );
        assert!(
            left_after_remove.block_light(uvec3(15, 8, 8))
                < left_after_remove.block_light(uvec3(15, 10, 8)),
            "neighbor light at removed emitter location must decrease"
        );
    }

    #[test]
    fn removal_of_all_emitters_clears_all_boundary_light() {
        let left_chunk = empty_chunk();
        let mut right_chunk = empty_chunk();
        right_chunk.blocks[0][8][8] = BlockType::Glowstone;

        let blocks: HashMap<IVec3, &Chunk> = HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

        let mut lights: HashMap<IVec3, ChunkLight> =
            HashMap::from([(ivec3(-1, 0, 0), ChunkLight::default())]);
        let mut right_light = ChunkLight::default();
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        let left_with = lights.remove(&ivec3(-1, 0, 0)).unwrap();
        assert_eq!(left_with.block_light(uvec3(15, 8, 8)), 14);

        right_chunk.blocks[0][8][8] = BlockType::Air;
        let mut lights: HashMap<IVec3, ChunkLight> = HashMap::from([(ivec3(-1, 0, 0), left_with)]);
        let mut dirty = 0;
        compute_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        clear_stale_neighbor_block_light(
            &right_chunk,
            &mut right_light,
            &blocks,
            &mut lights,
            &mut dirty,
        );
        let left_after = lights.remove(&ivec3(-1, 0, 0)).unwrap();

        for xz in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                assert_eq!(
                    left_after.block_light(uvec3(xz as u32, y as u32, 8)),
                    0,
                    "all neighbor boundary light must be 0 after removal; got light at ({xz},{y},8)"
                );
            }
        }
    }
}
