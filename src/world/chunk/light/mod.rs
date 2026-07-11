mod block;
mod propagation;
mod region;
mod sky;
mod storage;
#[cfg(test)]
mod tests;
mod utils;

#[cfg(test)]
use bevy::{platform::collections::HashMap, prelude::*};

#[cfg(test)]
use crate::block::BlockType;

pub use block::{clear_stale_neighbor_block_light, compute_block_light, pull_neighbor_block_light};
pub use region::compute_light_region;
pub use sky::compute_sky_light;
pub use storage::{ChunkHeightmap, ChunkLight};

#[cfg(test)]
use super::neighborhood::padded_chunk_index;
#[cfg(test)]
use bevy::platform::collections::HashSet;
#[cfg(test)]
use storage::{PADDED_LIGHT_WORDS, SKY_LIGHT_MAX};

#[cfg(test)]
use super::CHUNK_SIZE;
#[cfg(test)]
use super::{Chunk, ChunkCell};
