//! Vertex-pulling backend for terrain mesh layers.

mod pipeline;
mod prepare;
mod queue;

use bevy::{
    core_pipeline::core_3d::{Opaque3d, Transparent3d},
    ecs::schedule::IntoScheduleConfigs,
    render::{ExtractSchedule, Render, RenderSystems, render_phase::AddRenderCommand},
};

pub(super) fn install(render_app: &mut bevy::app::SubApp) {
    render_app
        .add_systems(
            ExtractSchedule,
            (
                prepare::extract_changed_meshes,
                prepare::extract_changed_lights,
            ),
        )
        .add_systems(
            Render,
            (
                prepare::prepare_gpu_data.in_set(RenderSystems::PrepareResources),
                queue::queue_chunk_meshes.in_set(RenderSystems::Queue),
            ),
        );
}

pub(super) fn finish(render_app: &mut bevy::app::SubApp) {
    render_app.add_render_command::<Opaque3d, queue::DrawChunkMeshCommands>();
    render_app.add_render_command::<Transparent3d, queue::DrawChunkMeshCommands>();
    pipeline::initialize(render_app);
}
