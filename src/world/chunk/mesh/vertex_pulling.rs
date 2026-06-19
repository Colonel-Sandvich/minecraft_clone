//! Vertex-pulling mesh generation and rendering — Phase 2.
//!
//! Non-greedy, texture-array terrain, per-vertex smooth lighting.
//! CPU emits an 8-byte `FaceDescriptor` per visible face. The vertex shader decodes
//! descriptors via `@builtin(vertex_index)` and samples the per-chunk light buffer.
//!
//! Bind group 0 (per frame): view_proj + terrain texture array + texture_layers
//! Bind group 1 (per chunk): face descriptor SSBO + chunk_origin + light_data

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::{
    asset::AssetId,
    camera::visibility::{self, VisibilityClass},
    core_pipeline::core_3d::{Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey},
    ecs::{
        change_detection::{Ref, Tick},
        component::Component,
        system::{
            SystemParamItem,
            lifetimeless::{Read, SRes},
        },
    },
    math::Mat4,
    prelude::*,
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        mesh::allocator::SlabId,
        render_asset::RenderAssets,
        render_phase::{
            AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
            RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
            ViewBinnedRenderPhases,
        },
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        sync_component::SyncComponentPlugin,
        sync_world::RenderEntity,
        texture::GpuImage,
        view::{ExtractedView, RenderVisibleEntities},
    },
};

use crate::block::BlockMaterialLayer;

use super::{
    CHUNK_SIZE, ChunkMeshBlocks, DIRECTION_COUNT, DIRECTION_INDEX_OFFSETS, block_mesh_flags,
    face_ao_key_from_indices, material_layer_index_from_flags, padded_chunk_index,
    should_emit_face_from_flags,
};

// ---------------------------------------------------------------------------
// Face descriptor (8 bytes, GPU-visible via bytemuck)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FaceDescriptor {
    pub packed: u32,
    pub info: u32,
}

impl FaceDescriptor {
    #[inline]
    pub fn new(x: u32, y: u32, z: u32, face_dir: u32, block_type: u32, ao_key: u32) -> Self {
        Self {
            packed: (x << 27) | (z << 22) | (y << 17) | (face_dir << 14),
            info: block_type | (ao_key << 8),
        }
    }
}

// ---------------------------------------------------------------------------
// Padded chunk blocks → non-greedy FaceDescriptors (per material layer)
// ---------------------------------------------------------------------------

pub fn build_descriptors(
    blocks: &ChunkMeshBlocks,
) -> Vec<(BlockMaterialLayer, Vec<FaceDescriptor>)> {
    if blocks.can_skip_mesh() {
        return Vec::new();
    }

    let capacity = blocks.center_rendered_blocks as usize;
    let mut descriptors: [Vec<FaceDescriptor>; BlockMaterialLayer::COUNT] =
        std::array::from_fn(|_| Vec::with_capacity(capacity));

    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let mut padded_index = padded_chunk_index(1, y + 1, z + 1);

            for x in 0..CHUNK_SIZE {
                let block = unsafe { *blocks.blocks.get_unchecked(padded_index) };
                let block_flags = block_mesh_flags(block);

                if block_flags != 0 {
                    for side_index in 0..DIRECTION_COUNT {
                        let neighbor_index =
                            (padded_index as isize + DIRECTION_INDEX_OFFSETS[side_index]) as usize;
                        let neighbor = unsafe { *blocks.blocks.get_unchecked(neighbor_index) };
                        let neighbor_flags = block_mesh_flags(neighbor);

                        if should_emit_face_from_flags(block, block_flags, neighbor, neighbor_flags)
                        {
                            let ao_key = face_ao_key_from_indices(blocks, padded_index, side_index);
                            descriptors[material_layer_index_from_flags(block_flags)].push(
                                FaceDescriptor::new(
                                    x as u32,
                                    y as u32,
                                    z as u32,
                                    side_index as u32,
                                    block as u32,
                                    ao_key,
                                ),
                            );
                        }
                    }
                }

                padded_index += 1;
            }
        }
    }

    BlockMaterialLayer::ALL
        .into_iter()
        .filter_map(|layer| {
            let mut layer_descriptors = std::mem::take(&mut descriptors[layer.index()]);
            layer_descriptors.shrink_to_fit();
            (!layer_descriptors.is_empty()).then_some((layer, layer_descriptors))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Main-world component
// ---------------------------------------------------------------------------

#[derive(Component, Clone)]
pub struct ChunkMeshDescriptors(pub Vec<FaceDescriptor>);

#[derive(Component, Clone)]
pub struct VertexPullingMesh {
    pub face_count: u32,
    pub material_layer: BlockMaterialLayer,
    pub chunk_origin: Vec3,
}

#[derive(Component, Clone)]
pub struct VertexPullingLight {
    pub light_data: Arc<[u32]>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SharedLightDataKey {
    ptr: usize,
    len: usize,
}

impl VertexPullingLight {
    pub fn data_key(&self) -> SharedLightDataKey {
        SharedLightDataKey {
            ptr: self.light_data.as_ptr() as usize,
            len: self.light_data.len(),
        }
    }
}

// ---------------------------------------------------------------------------
// Render-world resources
// ---------------------------------------------------------------------------

#[derive(Component)]
pub struct PreparedChunkVp {
    pub bind_group: BindGroup,
    pub face_count: u32,
    pub material_layer: BlockMaterialLayer,
    desc_buf: Buffer,
    origin_buf: Buffer,
    light_buf: Buffer,
}

#[derive(Resource)]
pub struct VpGlobals {
    pub group0_bind_group: BindGroup,
}

#[derive(Resource, Default)]
pub struct VpStaticResources {
    pub texture_layers_buf: Option<Buffer>,
    pub tint_colors_buf: Option<Buffer>,
    pub emission_factors_buf: Option<Buffer>,
    pub ao_brightness_buf: Option<Buffer>,
    pub visual_settings_buf: Option<Buffer>,
    pub view_proj_buf: Option<Buffer>,
}

#[derive(Resource)]
pub struct VpPipeline {
    pub chunk_bind_group_layout: BindGroupLayout,
    pub group0_bind_group_layout: BindGroupLayout,
    pub opaque_id: CachedRenderPipelineId,
    pub cutout_id: CachedRenderPipelineId,
}

#[derive(Resource, Clone)]
pub struct VpTextureState {
    pub terrain_texture_handle: Handle<Image>,
    pub texture_layers: Vec<u32>,
    pub tint_colors: Vec<[f32; 4]>,
    pub emission_factors: Vec<f32>,
    pub ao_brightness: [f32; 4],
}

impl ExtractResource for VpTextureState {
    type Source = VpTextureState;

    fn extract_resource(source: &Self::Source) -> Self {
        source.clone()
    }
}

#[derive(Resource, Reflect, Debug, Clone, Copy)]
#[reflect(Resource)]
pub struct TerrainVisualSettings {
    pub sky_light_color: Vec3,
    pub block_light_color: Vec3,
    pub fog_color: Vec3,
    pub fog_start: f32,
    pub fog_end: f32,
    pub fog_strength: f32,
}

impl Default for TerrainVisualSettings {
    fn default() -> Self {
        Self {
            sky_light_color: vec3(0.94, 0.97, 1.0),
            block_light_color: vec3(1.0, 0.78, 0.50),
            fog_color: vec3(0.455, 0.702, 1.0),
            fog_start: 220.0,
            fog_end: 560.0,
            fog_strength: 1.0,
        }
    }
}

impl ExtractResource for TerrainVisualSettings {
    type Source = TerrainVisualSettings;

    fn extract_resource(source: &Self::Source) -> Self {
        *source
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TerrainVisualSettingsUniform {
    sky_light_color: [f32; 4],
    block_light_color: [f32; 4],
    fog_color: [f32; 4],
    camera_position: [f32; 4],
    fog_params: [f32; 4],
}

impl TerrainVisualSettingsUniform {
    fn new(settings: TerrainVisualSettings, camera_position: Vec3) -> Self {
        Self {
            sky_light_color: [
                settings.sky_light_color.x,
                settings.sky_light_color.y,
                settings.sky_light_color.z,
                0.0,
            ],
            block_light_color: [
                settings.block_light_color.x,
                settings.block_light_color.y,
                settings.block_light_color.z,
                0.0,
            ],
            fog_color: [
                settings.fog_color.x,
                settings.fog_color.y,
                settings.fog_color.z,
                0.0,
            ],
            camera_position: [camera_position.x, camera_position.y, camera_position.z, 0.0],
            fog_params: [
                settings.fog_start,
                settings.fog_end,
                settings.fog_strength,
                0.0,
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// Chunk-origin uniform (16 bytes, for binding 1 in group 1)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ChunkOriginUniform {
    origin: [f32; 4], // Vec4 (xyz=origin, w=0)
}

// ---------------------------------------------------------------------------
// Draw command
// ---------------------------------------------------------------------------

pub struct DrawVertexPulled;

impl<P: PhaseItem> RenderCommand<P> for DrawVertexPulled {
    type Param = SRes<VpGlobals>;
    type ViewQuery = ();
    type ItemQuery = Read<PreparedChunkVp>;

    fn render<'w>(
        _item: &P,
        _view: (),
        data: Option<&'w PreparedChunkVp>,
        globals: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let globals = globals.into_inner();
        let Some(data) = data else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(0, &globals.group0_bind_group, &[]);
        pass.set_bind_group(1, &data.bind_group, &[]);
        pass.draw(0..data.face_count * 6, 0..1);
        RenderCommandResult::Success
    }
}

pub type DrawVpCmds = (SetItemPipeline, DrawVertexPulled);

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

const SHADER_PATH: &str = "shaders/vertex_pulling.wgsl";

pub struct VertexPullingPlugin;

impl Plugin for VertexPullingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TerrainVisualSettings>()
            .register_type::<TerrainVisualSettings>();
        app.register_required_components::<VertexPullingMesh, Transform>()
            .register_required_components::<VertexPullingMesh, Visibility>()
            .register_required_components::<VertexPullingMesh, VisibilityClass>();
        app.world_mut()
            .register_component_hooks::<VertexPullingMesh>()
            .on_add(visibility::add_visibility_class::<VertexPullingMesh>);

        app.add_plugins((
            SyncComponentPlugin::<VertexPullingMesh>::default(),
            ExtractResourcePlugin::<VpTextureState>::default(),
            ExtractResourcePlugin::<TerrainVisualSettings>::default(),
        ));

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_systems(
                ExtractSchedule,
                (extract_changed_vp_data, extract_changed_vp_lights),
            )
            .init_resource::<VpStaticResources>()
            .add_systems(
                Render,
                (
                    prepare_gpu_data.in_set(RenderSystems::PrepareResources),
                    queue_vp_meshes.in_set(RenderSystems::Queue),
                ),
            );
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app.add_render_command::<Opaque3d, DrawVpCmds>();

        let render_device = render_app.world().resource::<RenderDevice>().clone();

        let shader: Handle<Shader> = render_app
            .world_mut()
            .resource_mut::<AssetServer>()
            .load(SHADER_PATH);

        // Group 0: view_proj + terrain texture array + sampler + texture layers + tint colors + AO curve + emission factors + visual settings
        let group0_entries = vec![
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(64),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 5,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 6,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(16),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 7,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 8,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(std::mem::size_of::<
                        TerrainVisualSettingsUniform,
                    >() as u64),
                },
                count: None,
            },
        ];
        let group0_layout =
            render_device.create_bind_group_layout("vp_g0_globals", &group0_entries);

        // Group 1: faces SSBO + chunk_origin + light_data
        let group1_entries = vec![
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(16),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let group1_layout = render_device.create_bind_group_layout("vp_g1_chunk", &group1_entries);

        let group0_desc = BindGroupLayoutDescriptor {
            label: Cow::Borrowed("vp_g0_globals"),
            entries: group0_entries.clone(),
        };
        let group1_desc = BindGroupLayoutDescriptor {
            label: Cow::Borrowed("vp_g1_chunk"),
            entries: group1_entries.clone(),
        };

        let pipeline_cache = render_app.world().resource::<PipelineCache>();

        let multisample = MultisampleState {
            count: 4,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let multisample_alpha = MultisampleState {
            count: 4,
            mask: !0,
            alpha_to_coverage_enabled: true,
        };

        let opaque_id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(Cow::Borrowed("vp_opaque")),
            layout: vec![group0_desc.clone(), group1_desc.clone()],
            push_constant_ranges: vec![],
            vertex: VertexState {
                shader: shader.clone(),
                shader_defs: vec![],
                entry_point: Some(Cow::Borrowed("vertex")),
                buffers: vec![],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: Some(Face::Back),
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: CompareFunction::GreaterEqual,
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample,
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs: vec![],
                entry_point: Some(Cow::Borrowed("fragment")),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        });

        let cutout_id = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(Cow::Borrowed("vp_cutout")),
            layout: vec![group0_desc, group1_desc],
            push_constant_ranges: vec![],
            vertex: VertexState {
                shader: shader.clone(),
                shader_defs: vec![],
                entry_point: Some(Cow::Borrowed("vertex")),
                buffers: vec![],
            },
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: FrontFace::Ccw,
                cull_mode: Some(Face::Back),
                unclipped_depth: false,
                polygon_mode: PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: CompareFunction::GreaterEqual,
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: multisample_alpha,
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs: vec![],
                entry_point: Some(Cow::Borrowed("fragment")),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            zero_initialize_workgroup_memory: false,
        });

        render_app.world_mut().insert_resource(VpPipeline {
            chunk_bind_group_layout: group1_layout,
            group0_bind_group_layout: group0_layout.clone(),
            opaque_id,
            cutout_id,
        });

        // Create placeholder group 0 bind group (replaced each frame)
        let dummy_buf = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_view_proj"),
            contents: &[0u8; 64],
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let dummy_tex = render_device.create_texture(&TextureDescriptor {
            label: Some("vp_g0_dummy_tex"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let dummy_tex_view = dummy_tex.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            array_layer_count: Some(1),
            ..Default::default()
        });
        let dummy_sampler = render_device.create_sampler(&SamplerDescriptor::default());
        let dummy_texture_layers = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_texture_layers"),
            contents: bytemuck::cast_slice(&vec![0u32; 54]),
            usage: BufferUsages::STORAGE,
        });
        let dummy_tints = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_tints"),
            contents: bytemuck::cast_slice(&vec![0.0f32; 54 * 4]), // 54 vec4s
            usage: BufferUsages::STORAGE,
        });
        let dummy_ao = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_ao"),
            contents: bytemuck::cast_slice(&[1.0f32; 4]),
            usage: BufferUsages::UNIFORM,
        });
        let dummy_emissions = render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("vp_g0_dummy_emissions"),
            contents: bytemuck::cast_slice(&vec![0.0f32; 54]),
            usage: BufferUsages::STORAGE,
        });
        let dummy_visual_settings =
            TerrainVisualSettingsUniform::new(TerrainVisualSettings::default(), Vec3::ZERO);
        let dummy_visual_settings_buf =
            render_device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("vp_g0_dummy_visual_settings"),
                contents: bytemuck::bytes_of(&dummy_visual_settings),
                usage: BufferUsages::UNIFORM,
            });
        let dummy_group0 = render_device.create_bind_group(
            "vp_g0_dummy",
            &group0_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: dummy_buf.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&dummy_tex_view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&dummy_sampler),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: dummy_texture_layers.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: dummy_tints.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: dummy_ao.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: dummy_emissions.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: dummy_visual_settings_buf.as_entire_binding(),
                },
            ],
        );
        render_app.world_mut().insert_resource(VpGlobals {
            group0_bind_group: dummy_group0,
        });
    }
}

// ---------------------------------------------------------------------------
// Systems (module-level functions so `in_set` works)
// ---------------------------------------------------------------------------

fn extract_changed_vp_data(
    mut commands: Commands,
    meshes: Extract<Query<(RenderEntity, &VertexPullingMesh), Changed<VertexPullingMesh>>>,
    descriptors: Extract<
        Query<(RenderEntity, &ChunkMeshDescriptors), Changed<ChunkMeshDescriptors>>,
    >,
) {
    commands.try_insert_batch(
        meshes
            .iter()
            .map(|(entity, mesh)| (entity, mesh.clone()))
            .collect::<Vec<_>>(),
    );
    for (entity, desc) in &descriptors {
        commands.entity(entity).insert(desc.clone());
    }
}

fn extract_changed_vp_lights(
    mut commands: Commands,
    lights: Extract<Query<(RenderEntity, &VertexPullingLight), Changed<VertexPullingLight>>>,
) {
    commands.try_insert_batch(
        lights
            .iter()
            .map(|(entity, light)| (entity, light.clone()))
            .collect::<Vec<_>>(),
    );
}

fn prepare_gpu_data(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    pipeline: Option<Res<VpPipeline>>,
    texture_state: Option<Res<VpTextureState>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    chunks_q: Query<
        (Entity, &VertexPullingMesh, Option<Ref<VertexPullingLight>>),
        Changed<VertexPullingMesh>,
    >,
    descriptors_q: Query<&ChunkMeshDescriptors>,
    lights_q: Query<(Entity, Ref<VertexPullingLight>), Changed<VertexPullingLight>>,
    all_meshes_q: Query<&VertexPullingMesh>,
    prepared_q: Query<&PreparedChunkVp>,
    cameras_q: Query<&ExtractedView>,
    visual_settings: Option<Res<TerrainVisualSettings>>,
    mut globals: ResMut<VpGlobals>,
    mut static_res: ResMut<VpStaticResources>,
) {
    let Some(pipeline) = pipeline else { return };

    let view = cameras_q
        .iter()
        .find(|v| v.clip_from_view.col(2).w.abs() > 0.5)
        .or_else(|| cameras_q.iter().next());
    let view_proj = view
        .map(|v| {
            v.clip_from_world.unwrap_or_else(|| {
                let view_from_world = v.world_from_view.affine().inverse();
                v.clip_from_view * Mat4::from(view_from_world)
            })
        })
        .unwrap_or(Mat4::IDENTITY);
    let camera_position = view
        .map(|v| v.world_from_view.translation())
        .unwrap_or(Vec3::ZERO);
    let visual_settings = visual_settings
        .map(|settings| *settings)
        .unwrap_or_default();
    let visual_settings_uniform =
        TerrainVisualSettingsUniform::new(visual_settings, camera_position);

    let Some(texture_state) = texture_state.as_deref() else {
        return;
    };
    let Some(gpu_image) = gpu_images.get(&texture_state.terrain_texture_handle) else {
        return;
    };

    // Create static buffers once and keep them forever
    if static_res.view_proj_buf.is_none() {
        static_res.texture_layers_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_texture_layers"),
                contents: bytemuck::cast_slice(&texture_state.texture_layers),
                usage: BufferUsages::STORAGE,
            },
        ));

        let tints_flat: Vec<f32> = texture_state
            .tint_colors
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        static_res.tint_colors_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_tint_colors"),
                contents: bytemuck::cast_slice(&tints_flat),
                usage: BufferUsages::STORAGE,
            },
        ));

        static_res.emission_factors_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_emission_factors"),
                contents: bytemuck::cast_slice(&texture_state.emission_factors),
                usage: BufferUsages::STORAGE,
            },
        ));

        static_res.view_proj_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_view_proj"),
                contents: bytemuck::cast_slice(view_proj.as_ref()),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            },
        ));

        static_res.ao_brightness_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_ao_brightness"),
                contents: bytemuck::cast_slice(&texture_state.ao_brightness),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            },
        ));

        static_res.visual_settings_buf = Some(render_device.create_buffer_with_data(
            &BufferInitDescriptor {
                label: Some("vp_visual_settings"),
                contents: bytemuck::bytes_of(&visual_settings_uniform),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            },
        ));

        let group0_entries: [BindGroupEntry; 8] = [
            BindGroupEntry {
                binding: 0,
                resource: static_res
                    .view_proj_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
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
                resource: static_res
                    .texture_layers_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 5,
                resource: static_res
                    .tint_colors_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 6,
                resource: static_res
                    .ao_brightness_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 7,
                resource: static_res
                    .emission_factors_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
            BindGroupEntry {
                binding: 8,
                resource: static_res
                    .visual_settings_buf
                    .as_ref()
                    .unwrap()
                    .as_entire_binding(),
            },
        ];

        globals.group0_bind_group = render_device.create_bind_group(
            "vp_g0_globals",
            &pipeline.group0_bind_group_layout,
            &group0_entries,
        );
    } else {
        // Update view_proj buffer in-place (same buffer, same bind group)
        render_queue.0.write_buffer(
            static_res.view_proj_buf.as_ref().unwrap(),
            0,
            bytemuck::cast_slice(view_proj.as_ref()),
        );
    }

    render_queue.0.write_buffer(
        static_res.ao_brightness_buf.as_ref().unwrap(),
        0,
        bytemuck::cast_slice(&texture_state.ao_brightness),
    );
    render_queue.0.write_buffer(
        static_res.visual_settings_buf.as_ref().unwrap(),
        0,
        bytemuck::bytes_of(&visual_settings_uniform),
    );

    let mut mesh_prepared = HashSet::new();
    let mut prepared_light_buffers = HashMap::new();

    // Mesh changes rebuild descriptor/origin buffers. Existing light buffers are reused unless
    // this entity's light component changed in the same extract pass.
    for (entity, mesh, light) in &chunks_q {
        mesh_prepared.insert(entity);

        if mesh.face_count == 0 {
            commands.entity(entity).remove::<PreparedChunkVp>();
            continue;
        }

        let desc_buf = if let Ok(desc) = descriptors_q.get(entity) {
            create_descriptor_buffer_from_slice(&render_device, &desc.0)
        } else if let Ok(prepared) = prepared_q.get(entity) {
            prepared.desc_buf.clone()
        } else {
            continue;
        };
        let origin_buf = create_origin_buffer(&render_device, mesh);
        let light_buf = match (light.as_ref(), prepared_q.get(entity).ok()) {
            (Some(light), _) if light.is_changed() => {
                light_buffer_for(&render_device, &mut prepared_light_buffers, light, None)
            }
            (Some(light), Some(prepared)) => light_buffer_for(
                &render_device,
                &mut prepared_light_buffers,
                light,
                Some(&prepared.light_buf),
            ),
            (Some(light), _) => {
                light_buffer_for(&render_device, &mut prepared_light_buffers, light, None)
            }
            (None, Some(prepared)) => prepared.light_buf.clone(),
            (None, _) => continue,
        };
        let bind_group = create_chunk_bind_group(
            &render_device,
            &pipeline,
            &desc_buf,
            &origin_buf,
            &light_buf,
        );

        commands.entity(entity).remove::<ChunkMeshDescriptors>();
        commands.entity(entity).insert(PreparedChunkVp {
            bind_group,
            face_count: mesh.face_count,
            material_layer: mesh.material_layer,
            desc_buf,
            origin_buf,
            light_buf,
        });
    }

    // Light-only changes reuse descriptor/origin buffers and rebuild only the light buffer plus
    // bind group. If a mesh was prepared above, it already consumed the latest light data.
    for (entity, light) in &lights_q {
        if mesh_prepared.contains(&entity) {
            continue;
        }

        let light_buf = light_buffer_for(&render_device, &mut prepared_light_buffers, &light, None);
        let (desc_buf, origin_buf, face_count, material_layer) =
            if let Ok(prepared) = prepared_q.get(entity) {
                (
                    prepared.desc_buf.clone(),
                    prepared.origin_buf.clone(),
                    prepared.face_count,
                    prepared.material_layer,
                )
            } else if let Ok(desc) = descriptors_q.get(entity) {
                let Ok(mesh) = all_meshes_q.get(entity) else {
                    continue;
                };
                if mesh.face_count == 0 {
                    commands.entity(entity).remove::<PreparedChunkVp>();
                    continue;
                }
                (
                    create_descriptor_buffer_from_slice(&render_device, &desc.0),
                    create_origin_buffer(&render_device, mesh),
                    mesh.face_count,
                    mesh.material_layer,
                )
            } else {
                continue;
            };
        let bind_group = create_chunk_bind_group(
            &render_device,
            &pipeline,
            &desc_buf,
            &origin_buf,
            &light_buf,
        );

        commands.entity(entity).insert(PreparedChunkVp {
            bind_group,
            face_count,
            material_layer,
            desc_buf,
            origin_buf,
            light_buf,
        });
    }
}

fn create_descriptor_buffer_from_slice(
    render_device: &RenderDevice,
    descriptors: &[FaceDescriptor],
) -> Buffer {
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_desc"),
        contents: bytemuck::cast_slice(descriptors),
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    })
}

fn create_origin_buffer(render_device: &RenderDevice, mesh: &VertexPullingMesh) -> Buffer {
    let origin_uniform = ChunkOriginUniform {
        origin: [
            mesh.chunk_origin.x,
            mesh.chunk_origin.y,
            mesh.chunk_origin.z,
            0.0,
        ],
    };
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_origin"),
        contents: bytemuck::bytes_of(&origin_uniform),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    })
}

fn create_light_buffer(render_device: &RenderDevice, light: &VertexPullingLight) -> Buffer {
    render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("vp_light"),
        contents: bytemuck::cast_slice(light.light_data.as_ref()),
        usage: BufferUsages::STORAGE,
    })
}

fn light_buffer_for(
    render_device: &RenderDevice,
    prepared_light_buffers: &mut HashMap<SharedLightDataKey, Buffer>,
    light: &VertexPullingLight,
    reusable_existing: Option<&Buffer>,
) -> Buffer {
    let key = light.data_key();
    if let Some(buffer) = prepared_light_buffers.get(&key) {
        return buffer.clone();
    }

    if let Some(buffer) = reusable_existing {
        prepared_light_buffers.insert(key, buffer.clone());
        return buffer.clone();
    }

    let buffer = create_light_buffer(render_device, light);
    prepared_light_buffers.insert(key, buffer.clone());
    buffer
}

fn create_chunk_bind_group(
    render_device: &RenderDevice,
    pipeline: &VpPipeline,
    desc_buf: &Buffer,
    origin_buf: &Buffer,
    light_buf: &Buffer,
) -> BindGroup {
    render_device.create_bind_group(
        "vp_chunk",
        &pipeline.chunk_bind_group_layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: desc_buf.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: origin_buf.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: light_buf.as_entire_binding(),
            },
        ],
    )
}

fn queue_vp_meshes(
    pipeline: Option<Res<VpPipeline>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    views: Query<(&ExtractedView, &RenderVisibleEntities)>,
    prepared_chunks: Query<&PreparedChunkVp>,
    draw_fns: Res<DrawFunctions<Opaque3d>>,
) {
    let Some(pipeline) = pipeline.as_deref() else {
        return;
    };
    if prepared_chunks.is_empty() {
        return;
    }

    let draw_fn = draw_fns.read().id::<DrawVpCmds>();
    let opaque_batch_set_key = Opaque3dBatchSetKey {
        pipeline: pipeline.opaque_id,
        draw_function: draw_fn,
        material_bind_group_index: None,
        vertex_slab: SlabId::default(),
        index_slab: None,
        lightmap_slab: None,
    };
    let cutout_batch_set_key = Opaque3dBatchSetKey {
        pipeline: pipeline.cutout_id,
        ..opaque_batch_set_key.clone()
    };
    let bin_key = Opaque3dBinKey {
        asset_id: AssetId::<Mesh>::invalid().untyped(),
    };

    for (view, visible_entities) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        for &(entity, main_entity) in visible_entities.iter::<VertexPullingMesh>() {
            let Ok(data) = prepared_chunks.get(entity) else {
                continue;
            };
            let batch_set_key = match data.material_layer {
                BlockMaterialLayer::Opaque => opaque_batch_set_key.clone(),
                BlockMaterialLayer::Cutout => cutout_batch_set_key.clone(),
            };
            phase.add(
                batch_set_key,
                bin_key.clone(),
                (entity, main_entity),
                InputUniformIndex::default(),
                BinnedRenderPhaseType::NonMesh,
                Tick::default(),
            );
        }
    }
}
