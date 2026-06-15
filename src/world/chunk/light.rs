use std::collections::VecDeque;

use bevy::{platform::collections::HashMap, prelude::*};
use serde::{Deserialize, Serialize};

use crate::block::BlockType;

use super::{CHUNK_ISIZE, CHUNK_SIZE, Chunk};

const SKY_LIGHT_MAX: u8 = 15;

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
    if flat > 13 {
        flat - 1
    } else {
        flat
    }
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
    neighbor_lights: &HashMap<IVec3, &mut ChunkLight>,
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
    neighbor_lights: &HashMap<IVec3, &mut ChunkLight>,
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
    neighbor_lights: &mut HashMap<IVec3, &mut ChunkLight>,
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
    neighbor_lights: &mut HashMap<IVec3, &mut ChunkLight>,
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

fn neighbor_world(chunk_pos: IVec3, local: UVec3, dir_offset: IVec3) -> IVec3 {
    chunk_pos * CHUNK_ISIZE + local.as_ivec3() + dir_offset
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
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
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
            let n_world = neighbor_world(entry.chunk, entry.local, offset);
            let (n_chunk, n_local) = world_to_chunk_local(n_world);

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
                && write_sky_light(center_light, lights, center_pos, n_chunk, n_local, target, dirty_neighbors)
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
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
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
            let n_world = neighbor_world(entry.chunk, entry.local, offset);
            let (n_chunk, n_local) = world_to_chunk_local(n_world);

            let n_current =
                block_light_at(center_light, lights, center_pos, n_chunk, n_local);
            if n_current >= entry.level - 1 {
                continue;
            }

            let n_block = block_at(center_chunk, blocks, center_pos, n_chunk, n_local);
            let attenuation = n_block.light_opacity().max(1);

            let target = entry.level.saturating_sub(attenuation);
            if target > n_current
                && write_block_light(
                    center_light, lights, center_pos, n_chunk, n_local, target, dirty_neighbors,
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
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
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
            let n_world = neighbor_world(entry.chunk, entry.local, offset);
            let (n_chunk, n_local) = world_to_chunk_local(n_world);

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
                increase_queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: n_current,
                    directions: ALL_DIRECTIONS_BITSET,
                });
            }

            if write_sky_light(
                center_light, lights, center_pos, n_chunk, n_local, 0, dirty_neighbors,
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
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
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
            let n_world = neighbor_world(entry.chunk, entry.local, offset);
            let (n_chunk, n_local) = world_to_chunk_local(n_world);

            let n_current =
                block_light_at(center_light, lights, center_pos, n_chunk, n_local);
            if n_current == 0 {
                continue;
            }

            let n_block = block_at(center_chunk, blocks, center_pos, n_chunk, n_local);
            let attenuation = n_block.light_opacity().max(1);

            let target = entry.level.saturating_sub(attenuation);

            if n_current > target {
                increase_queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: n_current,
                    directions: ALL_DIRECTIONS_BITSET,
                });
            }

            let emitted = n_block.light_emission();
            if emitted > 0 {
                increase_queue.push_back(IncreaseEntry {
                    chunk: n_chunk,
                    local: n_local,
                    level: emitted,
                    directions: ALL_DIRECTIONS_BITSET,
                });
            }

            if write_block_light(
                center_light, lights, center_pos, n_chunk, n_local, 0, dirty_neighbors,
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
    lights: &HashMap<IVec3, &mut ChunkLight>,
    center_pos: IVec3,
    local: UVec3,
    level: u8,
) -> bool {
    for dir_idx in 0..6 {
        let offset = DIRECTION_OFFSETS[dir_idx];
        let n_world = neighbor_world(center_pos, local, offset);
        let (n_chunk, n_local) = world_to_chunk_local(n_world);
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
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
    dirty_neighbors: &mut u32,
    column_y: u32,
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
                    center_light.set_sky_light(
                        uvec3(x as u32, y as u32, z as u32),
                        current_sky,
                    );
                }
            }

            heightmap.heights[x][z] = highest;
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
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
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

// ── Full light rebuild ─────────────────────────────────────────────────────-

pub fn compute_light(
    center_chunk: &Chunk,
    center_light: &mut ChunkLight,
    heightmap: &mut ChunkHeightmap,
    blocks: &HashMap<IVec3, &Chunk>,
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
    dirty_neighbors: &mut u32,
    rendered: u16,
    column_y: u32,
) {
    // All-air chunk: sky light 15 everywhere, no block light, heightmap 0.
    if rendered == 0 {
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    center_light.light[x][z][y] =
                        (SKY_LIGHT_MAX << 4) | (center_light.light[x][z][y] & 0x0F);
                }
            }
        }
        return;
    }

    compute_sky_light(
        center_chunk,
        center_light,
        heightmap,
        blocks,
        lights,
        dirty_neighbors,
        column_y,
    );
    compute_block_light(center_chunk, center_light, blocks, lights, dirty_neighbors);
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
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
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
    lights: &mut HashMap<IVec3, &mut ChunkLight>,
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
        compute_sky_light(&chunk, &mut light, &mut heightmap, &blocks, &mut lights, &mut dirty, 0);

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
        compute_sky_light(&chunk, &mut light, &mut heightmap, &blocks, &mut lights, &mut dirty, 0);

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
        compute_sky_light(&chunk, &mut light, &mut heightmap, &blocks, &mut lights, &mut dirty, 0);

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
                sl,
                0,
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
        compute_sky_light(&chunk, &mut light, &mut heightmap, &blocks, &mut lights, &mut dirty, 0);

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

        let blocks: HashMap<IVec3, &Chunk> =
            HashMap::from([(ivec3(0, -1, 0), &lower_chunk)]);
        {
            let mut lights: HashMap<IVec3, &mut ChunkLight> =
                HashMap::from([(ivec3(0, -1, 0), &mut lower_light)]);
            let mut dirty = 0;
            compute_sky_light(
                &upper_chunk,
                &mut upper_light,
                &mut heightmap,
                &blocks,
                &mut lights,
                &mut dirty,
                0,
            );
        }

        assert!(upper_light.sky_light(uvec3(8, 0, 8)) > 0);
    }

    #[test]
    fn cross_chunk_block_light_propagates_between_chunks() {
        let left_chunk = Chunk::default();
        let mut left_light = ChunkLight::default();
        let mut right_chunk = Chunk::default();
        let mut right_light = ChunkLight::default();

        right_chunk.blocks[0][8][8] = BlockType::Glowstone;

        let blocks: HashMap<IVec3, &Chunk> =
            HashMap::from([(ivec3(-1, 0, 0), &left_chunk)]);

        {
            let mut lights: HashMap<IVec3, &mut ChunkLight> =
                HashMap::from([(ivec3(-1, 0, 0), &mut left_light)]);
            let mut dirty = 0;
            compute_block_light(&right_chunk, &mut right_light, &blocks, &mut lights, &mut dirty);
        }

        assert_eq!(right_light.block_light(uvec3(0, 8, 8)), 15);
        assert_eq!(right_light.block_light(uvec3(1, 8, 8)), 14);
        assert_eq!(left_light.block_light(uvec3(15, 8, 8)), 14);
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
        compute_sky_light(&chunk, &mut light, &mut heightmap, &blocks, &mut lights, &mut dirty, 0);

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                assert_eq!(heightmap.heights[x][z], 0);
            }
        }
    }
}
