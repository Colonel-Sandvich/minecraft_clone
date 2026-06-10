pub mod chunk;
pub mod dimension;

use bevy::prelude::*;

use chunk::ChunkPlugin;
use dimension::DimensionPlugin;

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((DimensionPlugin, ChunkPlugin));
    }
}
