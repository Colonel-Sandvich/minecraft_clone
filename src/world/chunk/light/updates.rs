use std::collections::VecDeque;

use bevy::{platform::collections::HashMap, prelude::*};

use super::super::Chunk;
use super::propagation::{
    ALL_DIRECTIONS_BITSET, DecreaseEntry, propagate_block_decrease, propagate_sky_decrease,
};
use super::storage::ChunkLight;
use super::utils::{block_at, block_light_at, sky_light_at, world_to_chunk_local};

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
