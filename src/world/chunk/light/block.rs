use std::collections::VecDeque;

use bevy::{platform::collections::HashMap, prelude::*};

use super::super::{CHUNK_SIZE, Chunk};
use super::propagation::{
    ALL_DIRECTIONS_BITSET, DIRECTION_OFFSETS, DecreaseEntry, IncreaseEntry,
    propagate_block_decrease, propagate_block_increase,
};
use super::storage::ChunkLight;
use super::utils::{face_local_pair, neighbor_chunk_local};

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
                let emission = center_chunk.hot_meta_xyz(x, y, z).light_emission;
                if emission > 0 {
                    let pos = uvec3(x as u32, y as u32, z as u32);
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

                let center_cell = center_chunk.get_cell(center_local);
                let attenuation = center_cell.light_opacity().max(1);
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
        );
    }
    *center_light = pre_decrease;
}
