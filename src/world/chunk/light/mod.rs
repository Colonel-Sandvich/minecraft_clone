mod block;
mod propagation;
mod region;
mod sky;
mod storage;
#[cfg(test)]
mod tests;
mod updates;
mod utils;

#[cfg(test)]
use bevy::{platform::collections::HashMap, prelude::*};

#[cfg(test)]
use crate::block::BlockType;

pub use block::{clear_stale_neighbor_block_light, compute_block_light, pull_neighbor_block_light};
pub use region::compute_light_region;
pub use sky::compute_sky_light;
pub use storage::{ChunkHeightmap, ChunkLight};
pub use updates::{light_on_place_block, light_on_place_sky};
pub use utils::world_to_chunk_local;

#[cfg(test)]
use bevy::platform::collections::HashSet;
#[cfg(test)]
use storage::{PADDED_CHUNK_LAYER_SIZE, PADDED_CHUNK_SIZE, PADDED_LIGHT_WORDS, SKY_LIGHT_MAX};

#[cfg(test)]
use super::CHUNK_SIZE;
#[cfg(test)]
use super::Chunk;
