use std::collections::{HashMap, HashSet};

use bevy::{
    math::Mat4,
    prelude::*,
    render::{
        Extract,
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        sync_world::RenderEntity,
        texture::GpuImage,
        view::ExtractedView,
    },
};

use super::{
    super::{
        material::TerrainMaterialState,
        visuals::{TerrainAnimationClock, TerrainVisualSettings, TerrainVisualSettingsUniform},
    },
    pipeline::{FrameBuffers, Globals, Pipeline},
};
use crate::world::chunk::mesh::{
    ChunkMeshFaces, ChunkMeshLayer, ChunkMeshLight, PackedFace, SharedLightDataKey,
};

#[derive(Component)]
pub(super) struct PreparedChunkMesh {
    pub(super) bind_group: BindGroup,
    pub(super) face_count: u32,
    pub(super) material_layer: crate::block::BlockMaterialLayer,
    pub(super) chunk_origin: Vec3,
    face_buffer: Buffer,
    light_buffer: Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ChunkOriginUniform {
    origin: [f32; 4],
}

struct MaterialBuffers {
    texture_layers: Buffer,
    tint_colors: Buffer,
    emission_factors: Buffer,
    ao_brightness: Buffer,
}

type MeshNeedsPreparation<Prepared = PreparedChunkMesh> = Or<(
    Changed<ChunkMeshLayer>,
    With<ChunkMeshFaces>,
    Without<Prepared>,
)>;

pub(super) fn extract_changed_meshes(
    mut commands: Commands,
    meshes: Extract<Query<(RenderEntity, &ChunkMeshLayer), Changed<ChunkMeshLayer>>>,
    faces: Extract<Query<(RenderEntity, &ChunkMeshFaces), Changed<ChunkMeshFaces>>>,
) {
    commands.try_insert_batch(
        meshes
            .iter()
            .map(|(entity, mesh)| (entity, mesh.clone()))
            .collect::<Vec<_>>(),
    );
    for (entity, faces) in &faces {
        commands.entity(entity).insert(faces.clone());
    }
}

pub(super) fn extract_changed_lights(
    mut commands: Commands,
    lights: Extract<Query<(RenderEntity, &ChunkMeshLight), Changed<ChunkMeshLight>>>,
) {
    commands.try_insert_batch(
        lights
            .iter()
            .map(|(entity, light)| (entity, light.clone()))
            .collect::<Vec<_>>(),
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_gpu_data(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline: Option<Res<Pipeline>>,
    material_state: Option<Res<TerrainMaterialState>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    changed_meshes: Query<
        (Entity, &ChunkMeshLayer, Option<Ref<ChunkMeshLight>>),
        MeshNeedsPreparation,
    >,
    faces: Query<&ChunkMeshFaces>,
    changed_lights: Query<(Entity, Ref<ChunkMeshLight>), Changed<ChunkMeshLight>>,
    all_meshes: Query<&ChunkMeshLayer>,
    prepared_meshes: Query<&PreparedChunkMesh>,
    cameras: Query<&ExtractedView>,
    visual_settings: Option<Res<TerrainVisualSettings>>,
    animation_clock: Option<Res<TerrainAnimationClock>>,
    mut globals: ResMut<Globals>,
) {
    let Some(pipeline) = pipeline else { return };
    let Some(material_state) = material_state else {
        return;
    };

    update_global_resources(
        &render_device,
        &render_queue,
        &pipeline,
        &material_state,
        &gpu_images,
        &cameras,
        visual_settings.as_deref().copied().unwrap_or_default(),
        animation_clock
            .as_deref()
            .map(|clock| clock.seconds)
            .unwrap_or_default(),
        &mut globals,
    );

    let mut prepared_this_frame = HashSet::new();
    let mut light_buffers_by_data = HashMap::new();
    let mut updated_light_buffers = HashSet::new();

    // Face payloads are an independent invalidation source. In particular, a topology rebuild may
    // keep the same face count and origin, so preparation must not rely on metadata changing.
    for (entity, mesh, light) in &changed_meshes {
        prepared_this_frame.insert(entity);

        if mesh.face_count() == 0 {
            commands.entity(entity).remove::<PreparedChunkMesh>();
            continue;
        }

        let face_buffer = if let Ok(faces) = faces.get(entity) {
            create_face_buffer(&render_device, faces.as_slice())
        } else if let Ok(prepared) = prepared_meshes.get(entity) {
            prepared.face_buffer.clone()
        } else {
            continue;
        };
        let existing = prepared_meshes.get(entity).ok();
        let light_buffer = if let Some(light) = light.as_ref() {
            let (buffer, created) = light_buffer_for(
                &render_device,
                &mut light_buffers_by_data,
                light,
                existing.map(|prepared| &prepared.light_buffer),
            );
            if light.is_changed() && !created {
                write_light_buffer_once(&render_queue, &mut updated_light_buffers, &buffer, light);
            }
            buffer
        } else if let Some(prepared) = existing {
            prepared.light_buffer.clone()
        } else {
            // Rendering waits until the independently extracted light component arrives.
            continue;
        };

        let prepared =
            create_prepared_mesh(&render_device, &pipeline, mesh, face_buffer, light_buffer);
        commands.entity(entity).remove::<ChunkMeshFaces>();
        commands.entity(entity).insert(prepared);
    }

    // A light-only change can update the fixed-size storage buffer in place. The bind group and
    // face/origin buffers continue to reference valid resources.
    for (entity, light) in &changed_lights {
        if prepared_this_frame.contains(&entity) {
            continue;
        }

        if let Ok(prepared) = prepared_meshes.get(entity) {
            write_light_buffer_once(
                &render_queue,
                &mut updated_light_buffers,
                &prepared.light_buffer,
                &light,
            );
            continue;
        }

        let Ok(faces) = faces.get(entity) else {
            continue;
        };
        let Ok(mesh) = all_meshes.get(entity) else {
            continue;
        };
        if mesh.face_count() == 0 {
            commands.entity(entity).remove::<PreparedChunkMesh>();
            continue;
        }

        let face_buffer = create_face_buffer(&render_device, faces.as_slice());
        let (light_buffer, _) =
            light_buffer_for(&render_device, &mut light_buffers_by_data, &light, None);
        let prepared =
            create_prepared_mesh(&render_device, &pipeline, mesh, face_buffer, light_buffer);
        commands.entity(entity).remove::<ChunkMeshFaces>();
        commands.entity(entity).insert(prepared);
    }
}

#[allow(clippy::too_many_arguments)]
fn update_global_resources(
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
    pipeline: &Pipeline,
    material_state: &Res<TerrainMaterialState>,
    gpu_images: &RenderAssets<GpuImage>,
    cameras: &Query<&ExtractedView>,
    visual_settings: TerrainVisualSettings,
    animation_seconds: f32,
    globals: &mut Globals,
) {
    let view = cameras
        .iter()
        .find(|view| view.clip_from_view.col(2).w.abs() > 0.5)
        .or_else(|| cameras.iter().next());
    let view_projection = view
        .map(|view| {
            view.clip_from_world.unwrap_or_else(|| {
                let view_from_world = view.world_from_view.affine().inverse();
                view.clip_from_view * Mat4::from(view_from_world)
            })
        })
        .unwrap_or(Mat4::IDENTITY);
    let camera_position = view
        .map(|view| view.world_from_view.translation())
        .unwrap_or(Vec3::ZERO);
    let visual_settings =
        TerrainVisualSettingsUniform::new(visual_settings, camera_position, animation_seconds);

    if material_state.is_changed() {
        globals.material_state_dirty = true;
    }
    if let Some(frame_buffers) = globals.frame_buffers.as_ref() {
        render_queue.0.write_buffer(
            &frame_buffers.view_projection,
            0,
            bytemuck::cast_slice(view_projection.as_ref()),
        );
        render_queue.0.write_buffer(
            &frame_buffers.visual_settings,
            0,
            bytemuck::bytes_of(&visual_settings),
        );
    } else {
        globals.frame_buffers = Some(create_frame_buffers(
            render_device,
            view_projection,
            &visual_settings,
        ));
    }

    if globals.material_state_dirty
        && let Some(gpu_image) = gpu_images.get(&material_state.terrain_texture_handle)
    {
        let material_buffers = create_material_buffers(render_device, material_state);
        globals.bind_group = create_global_bind_group(
            render_device,
            pipeline,
            gpu_image,
            globals.frame_buffers.as_ref().unwrap(),
            &material_buffers,
        );
        globals.material_state_dirty = false;
    }
}

fn create_frame_buffers(
    render_device: &RenderDevice,
    view_projection: Mat4,
    visual_settings: &TerrainVisualSettingsUniform,
) -> FrameBuffers {
    FrameBuffers {
        view_projection: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_view_proj"),
            contents: bytemuck::cast_slice(view_projection.as_ref()),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        }),
        visual_settings: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_visual_settings"),
            contents: bytemuck::bytes_of(visual_settings),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        }),
    }
}

fn create_material_buffers(
    render_device: &RenderDevice,
    material_state: &TerrainMaterialState,
) -> MaterialBuffers {
    MaterialBuffers {
        texture_layers: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_texture_layers"),
            contents: bytemuck::cast_slice(&material_state.texture_layers),
            usage: BufferUsages::STORAGE,
        }),
        tint_colors: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_tint_colors"),
            contents: bytemuck::cast_slice(&material_state.tint_colors),
            usage: BufferUsages::STORAGE,
        }),
        emission_factors: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_emission_factors"),
            contents: bytemuck::cast_slice(&material_state.emission_factors),
            usage: BufferUsages::STORAGE,
        }),
        ao_brightness: render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_ao_brightness"),
            contents: bytemuck::cast_slice(&material_state.ao_brightness),
            usage: BufferUsages::UNIFORM,
        }),
    }
}

fn create_global_bind_group(
    render_device: &RenderDevice,
    pipeline: &Pipeline,
    gpu_image: &GpuImage,
    frame_buffers: &FrameBuffers,
    material_buffers: &MaterialBuffers,
) -> BindGroup {
    render_device.create_bind_group(
        "vp_g0_globals",
        &pipeline.global_bind_group_layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: frame_buffers.view_projection.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&gpu_image.texture_view),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(&gpu_image.sampler),
            },
            BindGroupEntry {
                binding: 4,
                resource: material_buffers.texture_layers.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 5,
                resource: material_buffers.tint_colors.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 6,
                resource: material_buffers.ao_brightness.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 7,
                resource: material_buffers.emission_factors.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 8,
                resource: frame_buffers.visual_settings.as_entire_binding(),
            },
        ],
    )
}

fn create_prepared_mesh(
    render_device: &RenderDevice,
    pipeline: &Pipeline,
    mesh: &ChunkMeshLayer,
    face_buffer: Buffer,
    light_buffer: Buffer,
) -> PreparedChunkMesh {
    let origin_buffer = create_origin_buffer(render_device, mesh);
    let bind_group = create_chunk_bind_group(
        render_device,
        pipeline,
        &face_buffer,
        &origin_buffer,
        &light_buffer,
    );
    PreparedChunkMesh {
        bind_group,
        face_count: mesh.face_count(),
        material_layer: mesh.material_layer(),
        chunk_origin: mesh.origin(),
        face_buffer,
        light_buffer,
    }
}

fn create_face_buffer(render_device: &RenderDevice, faces: &[PackedFace]) -> Buffer {
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_desc"),
        contents: bytemuck::cast_slice(faces),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    })
}

fn create_origin_buffer(render_device: &RenderDevice, mesh: &ChunkMeshLayer) -> Buffer {
    let origin = mesh.origin();
    let origin = ChunkOriginUniform {
        origin: [origin.x, origin.y, origin.z, 0.0],
    };
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_origin"),
        contents: bytemuck::bytes_of(&origin),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    })
}

fn create_light_buffer(render_device: &RenderDevice, light: &ChunkMeshLight) -> Buffer {
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_light"),
        contents: bytemuck::cast_slice(light.data()),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    })
}

fn light_buffer_for(
    render_device: &RenderDevice,
    light_buffers_by_data: &mut HashMap<SharedLightDataKey, Buffer>,
    light: &ChunkMeshLight,
    reusable_existing: Option<&Buffer>,
) -> (Buffer, bool) {
    let key = light.data_key();
    if let Some(buffer) = light_buffers_by_data.get(&key) {
        return (buffer.clone(), false);
    }

    let (buffer, created) = reusable_existing
        .map(|buffer| (buffer.clone(), false))
        .unwrap_or_else(|| (create_light_buffer(render_device, light), true));
    light_buffers_by_data.insert(key, buffer.clone());
    (buffer, created)
}

fn write_light_buffer_once(
    render_queue: &RenderQueue,
    updated_light_buffers: &mut HashSet<BufferId>,
    buffer: &Buffer,
    light: &ChunkMeshLight,
) {
    if updated_light_buffers.insert(buffer.id()) {
        let bytes = bytemuck::cast_slice(light.data());
        debug_assert_eq!(buffer.size(), bytes.len() as u64);
        render_queue.0.write_buffer(buffer, 0, bytes);
    }
}

fn create_chunk_bind_group(
    render_device: &RenderDevice,
    pipeline: &Pipeline,
    face_buffer: &Buffer,
    origin_buffer: &Buffer,
    light_buffer: &Buffer,
) -> BindGroup {
    render_device.create_bind_group(
        "vp_chunk",
        &pipeline.chunk_bind_group_layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: face_buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: origin_buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: light_buffer.as_entire_binding(),
            },
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct PreparationVisits(usize);

    fn count_meshes_needing_preparation(
        meshes: Query<(Entity, &ChunkMeshLayer), MeshNeedsPreparation>,
        mut visits: ResMut<PreparationVisits>,
    ) {
        visits.0 += meshes.iter().count();
    }

    #[derive(Component)]
    struct AlreadyPrepared;

    fn count_pending_face_payloads(
        meshes: Query<(Entity, &ChunkMeshLayer), MeshNeedsPreparation<AlreadyPrepared>>,
        mut visits: ResMut<PreparationVisits>,
    ) {
        visits.0 += meshes.iter().count();
    }

    #[test]
    fn chunk_origin_uniform_matches_shader_layout() {
        assert_eq!(std::mem::size_of::<ChunkOriginUniform>(), 16);
    }

    #[test]
    fn unprepared_mesh_is_retried_after_its_change_tick_expires() {
        let mut app = App::new();
        app.init_resource::<PreparationVisits>()
            .add_systems(Update, count_meshes_needing_preparation);

        let faces = ChunkMeshFaces::new(Vec::new());
        app.world_mut().spawn(ChunkMeshLayer::new(
            crate::block::BlockMaterialLayer::Opaque,
            Vec3::ZERO,
            &faces,
        ));

        app.update();
        app.update();

        assert_eq!(app.world().resource::<PreparationVisits>().0, 2);
    }

    #[test]
    fn pending_face_payload_is_retried_for_an_existing_prepared_mesh() {
        let mut app = App::new();
        app.init_resource::<PreparationVisits>()
            .add_systems(Update, count_pending_face_payloads);

        let faces = ChunkMeshFaces::new(Vec::new());
        let mesh = ChunkMeshLayer::new(
            crate::block::BlockMaterialLayer::Opaque,
            Vec3::ZERO,
            &faces,
        );
        app.world_mut().spawn((mesh, faces, AlreadyPrepared));

        app.update();
        app.update();

        assert_eq!(app.world().resource::<PreparationVisits>().0, 2);
    }
}
