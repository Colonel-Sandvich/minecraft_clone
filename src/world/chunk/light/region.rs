use std::collections::VecDeque;

use bevy::{
    platform::collections::{HashMap, HashSet},
    prelude::*,
};

use crate::block::BlockType;

use super::super::{CHUNK_ISIZE, CHUNK_SIZE, Chunk};
use super::propagation::{ALL_DIRECTIONS_BITSET, DIRECTION_OFFSETS, IncreaseEntry};
use super::storage::{ChunkHeightmap, ChunkLight, SKY_LIGHT_MAX};
use super::utils::{face_local_pair, neighbor_chunk_local};

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

        for (dir_idx, &offset) in DIRECTION_OFFSETS.iter().enumerate() {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

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

        for (dir_idx, &offset) in DIRECTION_OFFSETS.iter().enumerate() {
            if entry.directions & (1 << dir_idx) == 0 {
                continue;
            }

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
