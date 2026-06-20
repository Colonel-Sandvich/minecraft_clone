use std::collections::VecDeque;

use bevy::{platform::collections::HashMap, prelude::*};

use super::super::Chunk;
use super::storage::ChunkLight;
use super::utils::{
    block_at, block_light_at, neighbor_chunk_local, sky_light_at, write_block_light,
    write_sky_light,
};

/// Direction order: pairs at indices (0,1), (2,3), (4,5) are opposites.
/// Opposite of idx = idx ^ 1.
pub(super) const DIRECTION_OFFSETS: [IVec3; 6] = [
    IVec3::X,
    IVec3::NEG_X,
    IVec3::Z,
    IVec3::NEG_Z,
    IVec3::Y,
    IVec3::NEG_Y,
];
pub(super) const ALL_DIRECTIONS_BITSET: u8 = 0b111111;

pub(super) struct IncreaseEntry {
    pub(super) chunk: IVec3,
    pub(super) local: UVec3,
    pub(super) level: u8,
    /// Bitmask of direction indices to propagate to.
    pub(super) directions: u8,
}

pub(super) struct DecreaseEntry {
    pub(super) chunk: IVec3,
    pub(super) local: UVec3,
    pub(super) level: u8,
    /// Bitmask of direction indices to propagate to.
    pub(super) directions: u8,
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

pub(super) fn propagate_sky_increase(
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

        for (dir_idx, &offset) in DIRECTION_OFFSETS.iter().enumerate() {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

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

pub(super) fn propagate_block_increase(
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

        for (dir_idx, &offset) in DIRECTION_OFFSETS.iter().enumerate() {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

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

// ── Block-light decrease propagation ────────────────────────────────────────

pub(super) fn propagate_block_decrease(
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
        for (dir_idx, &offset) in DIRECTION_OFFSETS.iter().enumerate() {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

            let (n_chunk, n_local) = neighbor_chunk_local(entry.chunk, entry.local, offset);

            let n_current = block_light_at(center_light, lights, center_pos, n_chunk, n_local);
            if n_current == 0 {
                continue;
            }

            let n_block = block_at(center_chunk, blocks, center_pos, n_chunk, n_local);
            let attenuation = n_block.light_opacity().max(1);

            let target = entry.level.saturating_sub(attenuation);

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
