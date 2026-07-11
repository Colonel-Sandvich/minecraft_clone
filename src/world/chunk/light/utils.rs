use bevy::{platform::collections::HashMap, prelude::*};

use super::super::{Chunk, ChunkCell, ChunkPos, LocalBlockPos, neighborhood::NeighborOffset};
use super::storage::{ChunkLight, SKY_LIGHT_MAX};

pub(super) fn face_local_pair(offset: IVec3, a: usize, b: usize) -> Option<(UVec3, UVec3)> {
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

pub(crate) fn offset_to_bit_index(offset: IVec3) -> u32 {
    NeighborOffset::try_new(offset)
        .expect("light neighbor offset must be adjacent and non-zero")
        .bit_index()
}

// ── Coordinate helpers ─────────────────────────────────────────────────────

pub(super) fn block_at(
    center_chunk: &Chunk,
    chunks: &HashMap<IVec3, &Chunk>,
    center_pos: IVec3,
    chunk_pos: IVec3,
    local: UVec3,
) -> ChunkCell {
    if chunk_pos == center_pos {
        center_chunk.get_cell(local)
    } else if let Some(chunk) = chunks.get(&chunk_pos) {
        chunk.get_cell(local)
    } else {
        ChunkCell::EMPTY
    }
}

// The &mut in the signature is intentional: callers hold &mut on the map
// for write_sky_light / write_block_light later, so a shared-reference
// re-borrow is needed here.
pub(super) fn sky_light_at(
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

pub(super) fn block_light_at(
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

pub(super) fn write_sky_light(
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

pub(super) fn write_block_light(
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

pub(super) fn neighbor_chunk_local(
    chunk_pos: IVec3,
    local: UVec3,
    offset: IVec3,
) -> (IVec3, UVec3) {
    let address = ChunkPos::from_ivec3(chunk_pos)
        .block(LocalBlockPos::try_from(local).expect("light position must be chunk-local"))
        .offset(offset);
    (address.chunk().as_ivec3(), address.local().as_uvec3())
}
