use std::collections::VecDeque;

use bevy::{platform::collections::HashMap, prelude::*};

use super::super::{CHUNK_SIZE, Chunk};
use super::propagation::{
    ALL_DIRECTIONS_BITSET, DIRECTION_OFFSETS, IncreaseEntry, propagate_sky_increase,
};
use super::storage::{ChunkHeightmap, ChunkLight, SKY_LIGHT_MAX};
use super::utils::{neighbor_chunk_local, sky_light_at};

fn has_darker_neighbor(
    center: &ChunkLight,
    lights: &HashMap<IVec3, ChunkLight>,
    center_pos: IVec3,
    local: UVec3,
    level: u8,
) -> bool {
    for &offset in &DIRECTION_OFFSETS {
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
