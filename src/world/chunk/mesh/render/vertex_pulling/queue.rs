use std::any::TypeId;

use bevy::{
    asset::AssetId,
    core_pipeline::core_3d::{
        Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey, Transparent3d, TransparentSortingInfo3d,
    },
    ecs::system::{
        SystemParamItem,
        lifetimeless::{Read, SRes},
    },
    prelude::*,
    render::{
        mesh::allocator::MeshSlabs,
        render_phase::{
            BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
            PhaseItemExtraIndex, RenderCommand, RenderCommandResult, SetItemPipeline,
            TrackedRenderPass, ViewBinnedRenderPhases, ViewSortedRenderPhases,
        },
        view::{ExtractedView, RenderVisibleEntities},
    },
};

use crate::{
    block::BlockMaterialLayer,
    world::chunk::{CHUNK_SIZE, mesh::ChunkMeshLayer},
};

use super::{
    pipeline::{Globals, Pipeline},
    prepare::PreparedChunkMesh,
};

pub(super) struct DrawVertexPulled;

impl<P: PhaseItem> RenderCommand<P> for DrawVertexPulled {
    type Param = SRes<Globals>;
    type ViewQuery = ();
    type ItemQuery = Read<PreparedChunkMesh>;

    fn render<'w>(
        _item: &P,
        _view: (),
        mesh: Option<&'w PreparedChunkMesh>,
        globals: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(mesh) = mesh else {
            return RenderCommandResult::Skip;
        };
        let globals = globals.into_inner();
        pass.set_bind_group(0, &globals.bind_group, &[]);
        pass.set_bind_group(1, &mesh.bind_group, &[]);
        pass.draw(0..mesh.face_count * 6, 0..1);
        RenderCommandResult::Success
    }
}

pub(super) type DrawChunkMeshCommands = (SetItemPipeline, DrawVertexPulled);

pub(super) fn queue_chunk_meshes(
    pipeline: Option<Res<Pipeline>>,
    mut opaque_phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    mut transparent_phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    views: Query<(&ExtractedView, &RenderVisibleEntities)>,
    prepared_meshes: Query<&PreparedChunkMesh>,
    opaque_draw_functions: Res<DrawFunctions<Opaque3d>>,
    transparent_draw_functions: Res<DrawFunctions<Transparent3d>>,
) {
    let Some(pipeline) = pipeline.as_deref() else {
        return;
    };
    if prepared_meshes.is_empty() {
        return;
    }

    let opaque_draw_function = opaque_draw_functions.read().id::<DrawChunkMeshCommands>();
    let transparent_draw_function = transparent_draw_functions
        .read()
        .id::<DrawChunkMeshCommands>();
    let opaque_batch_key = Opaque3dBatchSetKey {
        pipeline: pipeline.opaque_id,
        draw_function: opaque_draw_function,
        material_bind_group_index: None,
        slabs: MeshSlabs::default(),
        lightmap_slab: None,
    };
    let cutout_batch_key = Opaque3dBatchSetKey {
        pipeline: pipeline.cutout_id,
        ..opaque_batch_key.clone()
    };
    let bin_key = Opaque3dBinKey {
        asset_id: AssetId::<Mesh>::invalid().untyped(),
    };

    for (view, visible_entities) in &views {
        let Some(opaque_phase) = opaque_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Some(transparent_phase) = transparent_phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let rangefinder = view.rangefinder3d();
        let Some(visible_meshes) = visible_entities
            .classes
            .get(&TypeId::of::<ChunkMeshLayer>())
        else {
            continue;
        };

        for &(entity, main_entity) in &visible_meshes.entities_cpu_culling {
            let Ok(mesh) = prepared_meshes.get(entity) else {
                continue;
            };
            match mesh.material_layer {
                BlockMaterialLayer::Opaque => opaque_phase.add(
                    opaque_batch_key.clone(),
                    bin_key.clone(),
                    (entity, main_entity),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                ),
                BlockMaterialLayer::Cutout => opaque_phase.add(
                    cutout_batch_key.clone(),
                    bin_key.clone(),
                    (entity, main_entity),
                    InputUniformIndex::default(),
                    BinnedRenderPhaseType::NonMesh,
                ),
                BlockMaterialLayer::Translucent => {
                    let mesh_center = mesh.chunk_origin + Vec3::splat(CHUNK_SIZE as f32 * 0.5);
                    transparent_phase.add_retained(Transparent3d {
                        sorting_info: TransparentSortingInfo3d::Sorted {
                            mesh_center,
                            depth_bias: 0.0,
                        },
                        entity: (entity, main_entity),
                        pipeline: pipeline.translucent_id,
                        draw_function: transparent_draw_function,
                        distance: rangefinder.distance(&mesh_center),
                        batch_range: 0..1,
                        extra_index: PhaseItemExtraIndex::None,
                        indexed: false,
                    });
                }
            }
        }
    }
}
