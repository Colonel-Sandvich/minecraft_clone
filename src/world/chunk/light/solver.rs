use std::collections::VecDeque;

use bevy::platform::collections::HashSet;

use crate::quad::Direction;

use super::super::{CHUNK_ISIZE, CHUNK_SIZE, ChunkBlockPos, ChunkColumn, ChunkPos, LocalBlockPos};
use super::region::ChunkLightRegion;
use super::storage::SKY_LIGHT_MAX;

pub(super) fn rebuild(region: &mut ChunkLightRegion<'_>) {
    let calculation_positions = region.calculation_positions();
    let mut sky_queue = VecDeque::new();
    seed_sky_sources(region, &calculation_positions, &mut sky_queue);

    let mut block_queue = VecDeque::new();
    seed_block_sources(region, &calculation_positions, &mut block_queue);
    seed_boundary_light(
        region,
        &calculation_positions,
        &mut sky_queue,
        &mut block_queue,
    );

    propagate_sky_increase(region, &mut sky_queue);
    propagate_block_increase(region, &mut block_queue);
}

fn seed_sky_sources(
    region: &mut ChunkLightRegion<'_>,
    calculation_positions: &[ChunkPos],
    queue: &mut VecDeque<PropagationEntry>,
) {
    let columns = calculation_positions
        .iter()
        .copied()
        .map(ChunkColumn::from)
        .collect::<HashSet<_>>();
    let height_chunks = region.height_chunks() as i32;

    for column in columns {
        let top_chunk_loaded = region.contains_calculation(column.chunk(height_chunks - 1));
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                let mut current_sky = if top_chunk_loaded { SKY_LIGHT_MAX } else { 0 };
                let mut highest = 0u8;
                let mut found_highest = false;

                for chunk_y in (0..height_chunks).rev() {
                    let position = column.chunk(chunk_y);
                    let Some(chunk) = region.calculation_chunk(position) else {
                        current_sky = 0;
                        continue;
                    };

                    for y in (0..CHUNK_SIZE).rev() {
                        let meta = chunk.hot_meta_xyz(x, y, z);
                        if meta.light_opacity >= SKY_LIGHT_MAX {
                            if !found_highest {
                                highest = u8::try_from(chunk_y * CHUNK_ISIZE + y as i32)
                                    .expect("validated lighting height must fit the heightmap");
                                found_highest = true;
                            }
                            current_sky = 0;
                            continue;
                        }

                        current_sky = current_sky.saturating_sub(meta.light_opacity);
                        if current_sky > 0 {
                            region.write_sky_light(
                                position.block(LocalBlockPos::new(x as u32, y as u32, z as u32)),
                                current_sky,
                            );
                        }
                    }
                }

                for chunk_y in 0..height_chunks {
                    region.set_height(column.chunk(chunk_y), x, z, highest);
                }
            }
        }
    }

    for &position in calculation_positions {
        let Some(chunk) = region.calculation_chunk(position) else {
            continue;
        };

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let address = position.block(LocalBlockPos::new(x as u32, y as u32, z as u32));
                    let current = region.sky_light(address);
                    if current >= SKY_LIGHT_MAX - 1 {
                        continue;
                    }

                    let cell = chunk.cell(address.local());
                    if !cell.is_transparent_to_sky_light() {
                        continue;
                    }

                    let attenuation = cell.light_opacity().max(1);
                    let mut best = current;
                    for direction in Direction::ALL {
                        let neighbor = address.neighbor(direction);
                        if !region.contains_calculation(neighbor.chunk()) {
                            continue;
                        }
                        best = best.max(region.sky_light(neighbor).saturating_sub(attenuation));
                    }

                    if best > current && region.write_sky_light(address, best) {
                        queue.push_back(PropagationEntry::new(address, best));
                    }
                }
            }
        }
    }
}

fn seed_block_sources(
    region: &mut ChunkLightRegion<'_>,
    calculation_positions: &[ChunkPos],
    queue: &mut VecDeque<PropagationEntry>,
) {
    for &position in calculation_positions {
        let Some(chunk) = region.calculation_chunk(position) else {
            continue;
        };

        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for y in 0..CHUNK_SIZE {
                    let emission = chunk.hot_meta_xyz(x, y, z).light_emission;
                    if emission == 0 {
                        continue;
                    }

                    let address = position.block(LocalBlockPos::new(x as u32, y as u32, z as u32));
                    if region.write_block_light(address, emission) {
                        queue.push_back(PropagationEntry::new(address, emission));
                    }
                }
            }
        }
    }
}

fn seed_boundary_light(
    region: &mut ChunkLightRegion<'_>,
    calculation_positions: &[ChunkPos],
    sky_queue: &mut VecDeque<PropagationEntry>,
    block_queue: &mut VecDeque<PropagationEntry>,
) {
    for &position in calculation_positions {
        for direction in Direction::ALL {
            let neighbor_position = position.offset(direction.offset());
            if region.contains_calculation(neighbor_position) {
                continue;
            }

            for a in 0..CHUNK_SIZE {
                for b in 0..CHUNK_SIZE {
                    let (local, neighbor_local) = face_local_pair(direction, a, b);
                    let address = position.block(local);
                    let neighbor = neighbor_position.block(neighbor_local);
                    let sky_level = region.sky_light(neighbor);
                    let block_level = region.block_light(neighbor);
                    let block = region.cell(address);

                    if sky_level > 0 {
                        let attenuation = if block.is_transparent_to_sky_light() {
                            if direction == Direction::Up {
                                block.light_opacity()
                            } else {
                                block.light_opacity().max(1)
                            }
                        } else {
                            SKY_LIGHT_MAX
                        };
                        let level = sky_level.saturating_sub(attenuation);
                        if level > region.sky_light(address)
                            && region.write_sky_light(address, level)
                            && level > 1
                        {
                            sky_queue.push_back(PropagationEntry::new(address, level));
                        }
                    }

                    if block_level > 0 {
                        let level = block_level.saturating_sub(block.light_opacity().max(1));
                        if level > region.block_light(address)
                            && region.write_block_light(address, level)
                            && level > 1
                        {
                            block_queue.push_back(PropagationEntry::new(address, level));
                        }
                    }
                }
            }
        }
    }
}

fn propagate_sky_increase(
    region: &mut ChunkLightRegion<'_>,
    queue: &mut VecDeque<PropagationEntry>,
) {
    while let Some(entry) = queue.pop_front() {
        if entry.level <= 1 {
            continue;
        }

        for direction in Direction::ALL {
            if !entry.directions.contains(direction) {
                continue;
            }

            let neighbor = entry.address.neighbor(direction);
            if !region.contains_calculation(neighbor.chunk()) {
                continue;
            }

            let current = region.sky_light(neighbor);
            if current >= entry.level - 1 {
                continue;
            }

            let block = region.cell(neighbor);
            let attenuation = if block.is_transparent_to_sky_light() {
                block.light_opacity().max(1)
            } else {
                SKY_LIGHT_MAX
            };
            let level = entry.level.saturating_sub(attenuation);
            if level > current && region.write_sky_light(neighbor, level) {
                queue.push_back(PropagationEntry {
                    address: neighbor,
                    level,
                    directions: DirectionMask::ALL.without(direction.opposite()),
                });
            }
        }
    }
}

fn propagate_block_increase(
    region: &mut ChunkLightRegion<'_>,
    queue: &mut VecDeque<PropagationEntry>,
) {
    while let Some(entry) = queue.pop_front() {
        if entry.level <= 1 {
            continue;
        }

        for direction in Direction::ALL {
            if !entry.directions.contains(direction) {
                continue;
            }

            let neighbor = entry.address.neighbor(direction);
            if !region.contains_calculation(neighbor.chunk()) {
                continue;
            }

            let current = region.block_light(neighbor);
            if current >= entry.level - 1 {
                continue;
            }

            let level = entry
                .level
                .saturating_sub(region.cell(neighbor).light_opacity().max(1));
            if level > current && region.write_block_light(neighbor, level) {
                queue.push_back(PropagationEntry {
                    address: neighbor,
                    level,
                    directions: DirectionMask::ALL.without(direction.opposite()),
                });
            }
        }
    }
}

#[derive(Clone, Copy)]
struct PropagationEntry {
    address: ChunkBlockPos,
    level: u8,
    directions: DirectionMask,
}

impl PropagationEntry {
    const fn new(address: ChunkBlockPos, level: u8) -> Self {
        Self {
            address,
            level,
            directions: DirectionMask::ALL,
        }
    }
}

#[derive(Clone, Copy)]
struct DirectionMask(u8);

impl DirectionMask {
    const ALL: Self = Self((1 << Direction::COUNT) - 1);

    const fn contains(self, direction: Direction) -> bool {
        self.0 & (1 << direction.index()) != 0
    }

    const fn without(self, direction: Direction) -> Self {
        Self(self.0 & !(1 << direction.index()))
    }
}

fn face_local_pair(direction: Direction, a: usize, b: usize) -> (LocalBlockPos, LocalBlockPos) {
    let a = a as u32;
    let b = b as u32;
    match direction {
        Direction::Left => (
            LocalBlockPos::new(0, a, b),
            LocalBlockPos::new(CHUNK_SIZE as u32 - 1, a, b),
        ),
        Direction::Right => (
            LocalBlockPos::new(CHUNK_SIZE as u32 - 1, a, b),
            LocalBlockPos::new(0, a, b),
        ),
        Direction::Down => (
            LocalBlockPos::new(a, 0, b),
            LocalBlockPos::new(a, CHUNK_SIZE as u32 - 1, b),
        ),
        Direction::Up => (
            LocalBlockPos::new(a, CHUNK_SIZE as u32 - 1, b),
            LocalBlockPos::new(a, 0, b),
        ),
        Direction::Forward => (
            LocalBlockPos::new(a, b, 0),
            LocalBlockPos::new(a, b, CHUNK_SIZE as u32 - 1),
        ),
        Direction::Backward => (
            LocalBlockPos::new(a, b, CHUNK_SIZE as u32 - 1),
            LocalBlockPos::new(a, b, 0),
        ),
    }
}
