pub mod ambient_occlusion;
mod codec;
pub mod collider;
mod components;
mod coords;
mod data;
mod fluid_sim;
mod invalidation;
pub mod light;
pub mod mesh;
mod mutation;
pub(crate) mod neighborhood;
mod state;

use bevy::prelude::*;

use collider::ChunkColliderPlugin;
use mesh::ChunkMeshPlugin;

pub use codec::ChunkDecodeError;
pub use components::{
    ChunkContentCounts, ChunkNeedsColliderRebuild, ChunkNeedsFluidStep, ChunkNeedsLightRebuild,
    ChunkNeedsSave, ChunkPerfCounters, ChunkPosition,
};
pub use coords::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, ChunkBlockPos, ChunkColumn, ChunkIndex, ChunkPos,
    InvalidLocalBlockPos, LocalBlockPos, WorldBlockPos, chunk_linear_index,
};
pub use data::{CellStorage, Chunk, ChunkCellIter, ChunkPalette, ChunkRevision, PaletteEntry};
pub use fluid_sim::FluidStepResult;
pub(crate) use fluid_sim::{FluidSnapshot, simulate_fluid_step};
pub use invalidation::{ChunkInvalidationEffects, ChunkInvalidationPlan, classify_cell_delta};
pub use light::{ChunkHeightmap, ChunkLight};
pub use mutation::ChunkEditor;
pub use state::{
    AIR_CELL_STATE_ID, CELL_REGISTRY, CellDelta, CellRegistry, CellStateId, ChunkCell, FluidForm,
    FluidLevel, FluidProfile, FluidState, FluidType, HotCellMeta,
};

pub(crate) use neighborhood::chunk_neighbor_offsets;

pub struct ChunkPlugin;

impl Plugin for ChunkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChunkPerfCounters>()
            .add_plugins(ChunkMeshPlugin);
        if std::env::var_os("MINECRAFT_CLONE_DISABLE_CHUNK_COLLIDERS").is_none() {
            app.add_plugins(ChunkColliderPlugin);
        }
    }
}

#[cfg(test)]
mod tests;
