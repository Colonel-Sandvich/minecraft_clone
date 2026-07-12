use std::sync::Arc;

use bevy::{camera::primitives::Aabb, platform::collections::HashMap, prelude::*, utils::Parallel};

use crate::block::BlockMaterialLayer;
use crate::textures::TextureState;
use crate::world::dimension::{Active, Dimension};

use super::super::{
    CHUNK_SIZE, Chunk, ChunkLight, ChunkNeedsMeshRebuild, ChunkNeedsRenderLightUpload,
    ChunkPerfCounters, ChunkPos, ChunkPosition,
};
use super::{
    ChunkMeshBlocks, ChunkMeshFaces, ChunkMeshLayer, ChunkMeshLight, PreparedChunkMeshLight,
    mesher::{self, LayerMesh},
};

pub(super) fn install(app: &mut App) {
    app.add_systems(
        FixedPreUpdate,
        (rebuild_chunk_meshes, upload_chunk_lights)
            .chain()
            .run_if(in_state(TextureState::Finished)),
    )
    .add_systems(PostUpdate, drop_uploaded_faces);
}

pub(super) fn drop_uploaded_faces(
    mut commands: Commands,
    faces: Query<(Entity, Ref<ChunkMeshFaces>)>,
) {
    for (entity, faces_ref) in &faces {
        if !faces_ref.is_changed() {
            commands.entity(entity).remove::<ChunkMeshFaces>();
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn rebuild_chunk_meshes(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    all_chunks_q: Query<(&ChunkPosition, &Chunk)>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    children_q: Query<&Children>,
    mut mesh_q: Query<&mut ChunkMeshLayer>,
    mesh_light_q: Query<&ChunkMeshLight>,
    prepared_light_q: Query<&PreparedChunkMeshLight>,
    chunk_transform_q: Query<&Transform>,
    dimension: Option<Single<&Dimension, With<Active>>>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }

    let Some(dimension) = dimension else {
        return;
    };
    let dimension = dimension.into_inner();
    let active_dirty = dimension
        .iter_published_chunks()
        .filter_map(|(registered, entity)| {
            let (actual, _) = dirty_chunks_q.get(entity).ok()?;
            (actual.chunk_pos() == registered).then_some((entity, registered))
        })
        .collect::<HashMap<_, _>>();
    if active_dirty.is_empty() {
        return;
    }

    let mut chunks_by_pos = HashMap::with_capacity(dimension.loaded_chunk_count());
    for (registered, entity) in dimension.iter_loaded_chunks() {
        let Ok((actual, chunk)) = all_chunks_q.get(entity) else {
            continue;
        };
        if actual.chunk_pos() == registered {
            chunks_by_pos.insert(registered.as_ivec3(), chunk);
        }
    }

    let mut lights_by_pos = HashMap::default();
    if active_dirty
        .keys()
        .any(|&entity| prepared_light_q.get(entity).is_err())
    {
        lights_by_pos.reserve(dimension.loaded_chunk_count());
        for (registered, entity) in dimension.iter_loaded_chunks() {
            let Ok((actual, light)) = light_q.get(entity) else {
                continue;
            };
            if actual.chunk_pos() == registered {
                lights_by_pos.insert(registered, light);
            }
        }
    }

    let mut build_queue = Parallel::<Vec<ChunkMeshBuild>>::default();
    dirty_chunks_q.par_iter().for_each_init(
        || build_queue.borrow_local_mut(),
        |builds, (chunk_pos, chunk_entity)| {
            let Some(&position) = active_dirty.get(&chunk_entity) else {
                return;
            };
            debug_assert_eq!(chunk_pos.chunk_pos(), position);
            let blocks = ChunkMeshBlocks::from_chunks(position.as_ivec3(), &chunks_by_pos);
            builds.push(ChunkMeshBuild {
                entity: chunk_entity,
                chunk_pos: position,
                layers: mesher::build(&blocks),
            });
        },
    );

    let mut builds = Vec::new();
    build_queue.drain_into(&mut builds);
    let rebuilt_count = builds.len();
    for build in builds {
        let origin = chunk_transform_q
            .get(build.entity)
            .map(|transform| transform.translation)
            .unwrap_or(Vec3::ZERO);
        update_chunk_mesh_children(
            &mut commands,
            &mut mesh_q,
            build.entity,
            build.chunk_pos,
            children_q.get(build.entity).ok(),
            build.layers,
            origin,
            &lights_by_pos,
            &mesh_light_q,
            prepared_light_q.get(build.entity).ok(),
        );
        commands
            .entity(build.entity)
            .remove::<ChunkNeedsMeshRebuild>();
    }

    if let Some(perf) = perf.as_deref_mut() {
        perf.mesh_rebuilds += rebuilt_count;
    }
}

struct ChunkMeshBuild {
    entity: Entity,
    chunk_pos: ChunkPos,
    layers: Vec<LayerMesh>,
}

#[allow(clippy::too_many_arguments)]
fn update_chunk_mesh_children(
    commands: &mut Commands,
    mesh_q: &mut Query<&mut ChunkMeshLayer>,
    chunk_entity: Entity,
    chunk_pos: ChunkPos,
    children: Option<&Children>,
    layers: Vec<LayerMesh>,
    chunk_origin: Vec3,
    lights_by_pos: &HashMap<ChunkPos, &ChunkLight>,
    mesh_light_q: &Query<&ChunkMeshLight>,
    prepared_light: Option<&PreparedChunkMeshLight>,
) {
    let existing = children
        .map(|children| {
            children
                .iter()
                .filter_map(|entity| {
                    mesh_q
                        .get(entity)
                        .ok()
                        .map(|mesh| (mesh.material_layer(), entity))
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut updated = Vec::with_capacity(layers.len());
    let mut shared_light_data = prepared_light
        .map(PreparedChunkMeshLight::shared_data)
        .or_else(|| existing_child_light_data(&existing, mesh_light_q));
    for LayerMesh {
        material_layer,
        faces,
    } in layers
    {
        updated.push(material_layer);
        let faces = ChunkMeshFaces::new(faces);

        if let Some(entity) = existing.get(&material_layer) {
            if let Ok(mut mesh) = mesh_q.get_mut(*entity) {
                mesh.update(material_layer, chunk_origin, &faces);
            }
            commands
                .entity(*entity)
                .insert((faces, chunk_render_aabb()));
            continue;
        }

        let light_data =
            light_data_for_new_mesh_child(&mut shared_light_data, chunk_pos, lights_by_pos);
        let mesh = ChunkMeshLayer::new(material_layer, chunk_origin, &faces);

        commands.spawn((
            ChildOf(chunk_entity),
            Transform::default(),
            Visibility::default(),
            chunk_render_aabb(),
            mesh,
            faces,
            ChunkMeshLight::new(light_data),
        ));
    }

    for (layer, entity) in existing {
        if !updated.contains(&layer) {
            commands.entity(entity).despawn();
        }
    }
}

fn existing_child_light_data(
    existing: &HashMap<BlockMaterialLayer, Entity>,
    mesh_light_q: &Query<&ChunkMeshLight>,
) -> Option<Arc<[u32]>> {
    existing.values().find_map(|entity| {
        mesh_light_q
            .get(*entity)
            .ok()
            .map(ChunkMeshLight::shared_data)
    })
}

fn light_data_for_new_mesh_child(
    shared_light_data: &mut Option<Arc<[u32]>>,
    chunk_pos: ChunkPos,
    lights_by_pos: &HashMap<ChunkPos, &ChunkLight>,
) -> Arc<[u32]> {
    shared_light_data
        .get_or_insert_with(|| {
            Arc::from(ChunkMeshLight::build_padded_data(chunk_pos, lights_by_pos))
        })
        .clone()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn upload_chunk_lights(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), With<ChunkNeedsRenderLightUpload>>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    prepared_light_q: Query<&PreparedChunkMeshLight>,
    children_q: Query<&Children>,
    mut mesh_light_q: Query<&mut ChunkMeshLight>,
    dimension: Option<Single<&Dimension, With<Active>>>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }

    let Some(dimension) = dimension else {
        return;
    };
    let dimension = dimension.into_inner();
    let dirty_chunks = dimension
        .iter_published_chunks()
        .filter_map(|(registered, entity)| {
            let (actual, _) = dirty_chunks_q.get(entity).ok()?;
            (actual.chunk_pos() == registered).then_some((registered, entity))
        })
        .collect::<Vec<_>>();
    if dirty_chunks.is_empty() {
        return;
    }
    let upload_count = dirty_chunks.len();

    let mut lights_by_pos = HashMap::default();
    if dirty_chunks
        .iter()
        .any(|(_, entity)| prepared_light_q.get(*entity).is_err())
    {
        lights_by_pos.reserve(dimension.loaded_chunk_count());
        for (registered, entity) in dimension.iter_loaded_chunks() {
            let Ok((actual, light)) = light_q.get(entity) else {
                continue;
            };
            if actual.chunk_pos() == registered {
                lights_by_pos.insert(registered, light);
            }
        }
    }

    for (chunk_pos, chunk_entity) in dirty_chunks {
        let light_data = prepared_light_q
            .get(chunk_entity)
            .map(PreparedChunkMeshLight::shared_data)
            .unwrap_or_else(|_| {
                Arc::from(ChunkMeshLight::build_padded_data(chunk_pos, &lights_by_pos))
            });
        if let Ok(children) = children_q.get(chunk_entity) {
            for child in children {
                if let Ok(mut light) = mesh_light_q.get_mut(*child) {
                    light.replace(light_data.clone());
                }
            }
        }

        commands
            .entity(chunk_entity)
            .remove::<ChunkNeedsRenderLightUpload>();
    }

    if let Some(perf) = perf.as_deref_mut() {
        perf.light_uploads += upload_count;
    }
}

const fn chunk_render_aabb() -> Aabb {
    let half = CHUNK_SIZE as f32 / 2.0;
    let extent = vec3a(half, half, half);
    Aabb {
        center: extent,
        half_extents: extent,
    }
}
