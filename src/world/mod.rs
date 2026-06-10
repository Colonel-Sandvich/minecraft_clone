pub mod chunk;
pub mod dimension;

use avian3d::prelude::CollisionLayers;
use bevy::prelude::*;

use chunk::ChunkPlugin;
use dimension::DimensionPlugin;

pub struct WorldPlugin;

pub const WORLD_LAYER: u32 = 1 << 0;
pub const ACTOR_LAYER: u32 = 1 << 1;

pub const WORLD_COLLISION_LAYERS: CollisionLayers =
    CollisionLayers::from_bits(WORLD_LAYER, ACTOR_LAYER);
pub const ACTOR_COLLISION_LAYERS: CollisionLayers =
    CollisionLayers::from_bits(ACTOR_LAYER, WORLD_LAYER);

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((DimensionPlugin, ChunkPlugin));
    }
}
