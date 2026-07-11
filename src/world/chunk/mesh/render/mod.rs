//! Terrain mesh rendering.
//!
//! Main-world mesh components stay technique-neutral. The current backend uses
//! vertex pulling to decode compact face descriptors directly in the shader.

pub(super) mod material;
mod vertex_pulling;
mod visuals;

use bevy::{
    camera::visibility::{self, VisibilityClass},
    prelude::*,
    render::{
        RenderApp,
        extract_resource::ExtractResourcePlugin,
        sync_component::{SyncComponent, SyncComponentPlugin},
    },
};

use super::ChunkMeshLayer;
use material::TerrainMaterialState;
use visuals::{TerrainAnimationClock, TerrainVisualSettings};

pub(super) const VERTEX_PULLING_SHADER_SOURCE: &str =
    include_str!("../../../../../assets/shaders/vertex_pulling.wgsl");

pub(super) struct TerrainRenderPlugin;

impl Plugin for TerrainRenderPlugin {
    fn build(&self, app: &mut App) {
        material::install(app);
        visuals::install(app);

        app.register_required_components::<ChunkMeshLayer, Transform>()
            .register_required_components::<ChunkMeshLayer, Visibility>()
            .register_required_components::<ChunkMeshLayer, VisibilityClass>();
        app.world_mut()
            .register_component_hooks::<ChunkMeshLayer>()
            .on_add(visibility::add_visibility_class::<ChunkMeshLayer>);

        app.add_plugins((
            SyncComponentPlugin::<ChunkMeshLayer>::default(),
            ExtractResourcePlugin::<TerrainMaterialState>::default(),
            ExtractResourcePlugin::<TerrainVisualSettings>::default(),
            ExtractResourcePlugin::<TerrainAnimationClock>::default(),
        ));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        vertex_pulling::install(render_app);
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        vertex_pulling::finish(render_app);
    }
}

impl SyncComponent for ChunkMeshLayer {
    type Target = ChunkMeshLayer;
}
