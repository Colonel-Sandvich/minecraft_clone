//! Chunk meshing and terrain rendering.
//!
//! The mesher turns a padded chunk neighborhood into packed faces grouped by
//! material layer. Main-world systems own rebuild and lighting lifecycles, and
//! the private renderer currently consumes those faces through vertex pulling.

mod blocks;
mod components;
mod face;
mod light;
pub mod mesher;
mod render;
mod systems;

use bevy::prelude::*;

pub use blocks::ChunkMeshBlocks;
pub use components::{ChunkMeshFaces, ChunkMeshLayer, ChunkMeshLight};
pub use face::PackedFace;
pub use mesher::LayerMesh;

pub(crate) use blocks::DIRECTION_COUNT;
pub(crate) use components::{PreparedChunkMeshLight, SharedLightDataKey};

/// Shader source exposed for CPU/GPU contract validation.
pub const TERRAIN_SHADER_SOURCE: &str = render::VERTEX_PULLING_SHADER_SOURCE;

pub struct ChunkMeshPlugin;

impl Plugin for ChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        systems::install(app);
        app.add_plugins(render::TerrainRenderPlugin);
    }
}

#[cfg(test)]
pub(crate) use super::neighborhood::padded_chunk_index;
#[cfg(test)]
pub(crate) use mesher::{face_ao_from_indices, water_below_pair, water_corner_heights};

#[cfg(test)]
mod tests;
